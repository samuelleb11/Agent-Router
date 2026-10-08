# Web console contracts

2026-10-05. The behavioural rules agent-router's React and IBM Carbon console must keep while it
replaces `assets/index.html` (the dashboard, at `/`) and `assets/live.html` (the live view, at
`/live`). The plan is [Moving the web UI to React and IBM Carbon](web-console-carbon.md). The work
is Northstar's AGEN-0001.

This was written before any screen is rewritten, the way Northstar wrote its console contracts
before its own rewrite. Most rules come from the two pages' comments, which record why each rule
exists and which bug it took to learn it. The rest come from:

- the two check scripts;
- the phones' web-parity tests;
- [session liveness](session-liveness.md);
- the router's serving code;
- the Northstar contracts that bind here.

## How to read a contract

Each contract is one bullet:

- **The id**: two capitals for the area (SH Shell, OV Overview, SE Sessions, MA Machines, ST
  Settings, SV Serving), `-C` and a number.
- **The rule**: what the console must do, not how the old page did it.
- *Why:* the reason, from the comment, commit or check behind it.
- *Regression:* yes when it was a real bug once, with the evidence.
- *Source:* file and lines, at commit `7965562` unless stated.
- *Phones:* the iOS or Android test that mirrors the rule, where one does. Those rules hold on
  three clients, and changing one is a three-client decision.
- *Today:* broken, where the old page breaks its own rule. The console fixes it (decision G in
  the plan).

## How the slices use it

Each screen's slice in Northstar is done only when every id in its section is named in a spec or
a shot script under `ui/`, and the whole suite passes. The screen slices are AGEN-0010 (Overview),
AGEN-0011 (Settings) and AGEN-0012 (Sessions). A spec names the contract it pins in its title, as
in `it('OV-C68: red only for a crossing that survives a quiet week', …)`. A rule that holds
app-wide may appear in more than one section, on purpose, so that each screen's slice pins it
where it applies.

Counts at writing:

| Section | Contracts | Past regressions | Broken today |
| --- | ---: | ---: | ---: |
| Shell | 55 | 13 | 15 |
| Overview | 213 | 10 | 53 |
| Sessions | 218 | 47 | 43 |
| Settings | 103 | 24 | 25 |
| Serving | 44 | 7 | 4 |
| **All** | **633** | **101** | **140** |

Since then AGEN-0023 has added SE-C219 to SE-C225, for a hosted router's machines, so Sessions
holds 225. On 2026-10-07 the Machines screen added a section of its own, MA-C1 to MA-C7, and the
same day Sessions tabbed a hosted router's machines, SE-C226 to SE-C230, so it holds 230.

## Shell

What holds on every screen of the console: the topbar and the navigation, freshness, the
rules every panel obeys, tooltips, theme and motion, both widths, the keyboard, the URL and
the palette, and what the VS Code extension and the menubar app expect when they open it.
Paths are relative to this repository and `../northstar/` is the sibling Northstar
repository; line numbers are as of this repository's commit `7965562` and Northstar's
`09321b4`. *Today:* describes what the pages in `assets/` do now.

### Topbar and navigation

- **SH-C1** The topbar is Carbon g100 in both themes and holds, in this order, the menu
  button, the name "Agent Router", the freshness badge, a search action that opens the ⌘K
  palette, and the system/light/dark switch; it carries no sign-in or log-out action.
  *Why:* Decision A takes Northstar's shell as it stands, minus what a gate the browser
  handles itself (HTTP Basic) has no use for. *Regression:* no. *Source:*
  `docs/web-console-carbon.md:192-194`, `docs/web-console-carbon.md:545-548`,
  `docs/web-console-carbon.md:629`, `../northstar/ui/src/app/Shell.tsx:115-178`.
- **SH-C2** The topbar's global bar is bounded so its actions never push off a 390 px
  header, and the fixed header is padded by the safe-area inset so a notched phone never
  hides it. *Why:* Carbon's `HeaderGlobalBar` grows with its content, and Carbon's `Header`
  is `position: fixed` and ignores safe areas; today's live view pads its header by
  `env(safe-area-inset-top)`. *Regression:* no. *Source:*
  `docs/web-console-carbon.md:210-212`, `docs/web-console-carbon.md:328-329`,
  `docs/web-console-carbon.md:545-548`, `assets/live.html:38-44`.
- **SH-C3** The destinations are Overview, Sessions, on a hosted router Machines (MA-C1),
  then each project (opening the Overview scoped to it), with Settings pinned at the foot; a
  desk shows them in a rail, a phone in a drawer behind the menu button, with the same
  destinations in the same order.
  *Why:* Northstar §7 — only the container differs between widths; Settings at the foot so
  a growing project list never moves it. *Regression:* no. *Source:*
  `docs/web-console-carbon.md:195-200`, `../northstar/ui/src/app/Shell.tsx:64-67`,
  `../northstar/ui/src/app/Shell.tsx:213-225`, `../northstar/docs/13-console-contracts.md:299`.
- **SH-C4** The rail's collapsed or expanded state is remembered per browser, read and
  written inside try/catch; the phone drawer is never remembered and closes on its overlay,
  on Escape and on arriving anywhere. *Why:* a rail is a preference and a drawer is a
  moment; a private window must mean "not remembered", never a failed page. *Regression:*
  no. *Source:* `../northstar/ui/src/app/Shell.tsx:28-42`,
  `../northstar/ui/src/app/Shell.tsx:64-82`, `../northstar/ui/src/app/Shell.tsx:92-106`,
  `docs/web-console-carbon.md:196-199`.
- **SH-C5** Overview and Sessions are each one press away from every screen at both widths,
  including from inside an open session and from Find. *Why:* today the dashboard links to
  `/live` ("Follow and control running sessions") and the live list links back ("‹
  Dashboard"), but the live view hides that link while a session or Find is open, so the
  dashboard is two steps away there; the rail and the drawer replace both links.
  *Regression:* no. *Source:* `assets/index.html:399`, `assets/live.html:372`,
  `assets/live.html:1146-1147`, `assets/live.html:1396`, `assets/live.html:1436`.
- **SH-C6** A sentence on one screen that names a control on another links to that control:
  the Sessions notice that an editor "is not wrapped — sessions it starts are view-only until
  the wrapper is installed" links to Settings → Claude Code. *Why:* today it says "from the
  dashboard's Settings" in plain text, which gives directions instead of a way there.
  *Regression:* no. *Source:* `assets/live.html:624-655`.
- **SH-C7** Every screen sets a document title that names it: "Agent Router" for the
  Overview, "Live sessions · Agent Router" for Sessions, and its section for each Settings
  page. *Why:* browser tabs, the history list and VS Code's editor tabs are told apart by it;
  today's pages are titled that way, and the extension titles its fallback panels "Agent
  Router" and "Live sessions". *Regression:* no. *Source:* `assets/index.html:6`,
  `assets/live.html:4`, `vscode/src/extension.ts:18-19`.
- **SH-C8** A screen that throws renders an error screen inside the shell, whose topbar and
  navigation keep working, and an unknown path inside the app renders a not-found screen
  inside the shell, never a blank page. *Why:* Northstar's `App.tsx` gives every route an
  error element so a crashed screen leaves the shell alive, and a blank page reads as a dead
  router. *Regression:* no. *Source:* `docs/web-console-carbon.md:538`,
  `docs/web-console-carbon.md:556-558`, `../northstar/docs/13-console-contracts.md:332-333`.
- **SH-C9** The address the console was reached at (`location.host`) stays visible at both
  widths. *Why:* today's header prints it on every view, and it is how a reader tells which
  router a tab is showing — the Mac's, one on the LAN, or a hosted one. *Regression:* no.
  *Source:* `assets/index.html:56`, `assets/index.html:402`, `assets/index.html:995`.

### Freshness

- **SH-C10** The freshness badge has its own word and tone for each state: connecting
  (nothing has answered yet, neutral), live, stale (the last refresh failed; it names the
  time of the last good one, or says "never read successfully"), unreachable, and
  reconnecting (a Sessions stream being replaced); its accessible name is the whole
  sentence, announced as `role="status"`, never as an alert. *Why:* a state still finding
  out must not wear the live colour, and Carbon's `Tag` drops `title`, so the sentence has to
  be the name. *Regression:* yes — Northstar's `LiveBadge` once printed three words for four
  modes (`../northstar/ui/src/components/LiveBadge.tsx:9-13`). *Source:*
  `assets/index.html:398`, `assets/index.html:1128-1139`, `assets/live.html:68-69`,
  `assets/live.html:1122-1133`, `../northstar/ui/src/components/LiveBadge.tsx:5-27`,
  `../northstar/docs/13-console-contracts.md:44-45`. *Today:* broken — "connecting…" is
  drawn with the green `.dot`, and a page that never had an answer says "stale · last —".
- **SH-C11** The badge turns to unreachable only after five consecutive failed refresh
  cycles, counted per cycle and never per HTTP request, and one successful cycle sets the
  count back to zero. *Why:* one blip should read stale, not dead; a cycle reads eight or
  more endpoints at once, so counting requests would reach five on the first failure.
  *Regression:* no. *Source:* `assets/index.html:1131-1135`,
  `assets/index.html:2652-2657`, `docs/web-console-carbon.md:234-236`.
- **SH-C12** A cycle fails when a core read fails; the optional reads (quota, forecast and
  auth) failing on their own never count as a strike, never blank their panel, and leave
  their last good value showing. *Why:* "Optional: present only on builds that carry it,
  and a failure must not blank the page." *Regression:* no. *Source:*
  `assets/index.html:2507-2523`, `assets/index.html:2529-2530`.
- **SH-C13** Every request has a timeout, and a read that times out fails its cycle like any
  other failure. *Why:* the refresh loop schedules the next cycle only after the previous
  one settles, so a request that never answers freezes the page under a badge still saying
  "live". *Regression:* no. *Source:* `assets/index.html:809-820`,
  `assets/index.html:2652-2657`, `docs/web-console-carbon.md:238`. *Today:* broken — `get()`
  and `post()` have no timeout; a hung read stops every refresh and the badge keeps its last
  word.
- **SH-C14** A failed refresh never blanks what is on screen: every panel keeps its last data
  and the badge says how old it is. *Why:* Northstar §1 — losing what you were reading to a
  blip is worse than seeing it stale and marked. *Regression:* no. *Source:*
  `assets/index.html:2652-2657`, `../northstar/docs/13-console-contracts.md:33`,
  `../northstar/ui/src/lib/freshness.ts:45-52`.
- **SH-C15** One clock drives every re-read: the Overview refetches five seconds after the
  previous read completes, cycles never overlap and never retry quietly, auth and accounts
  are re-read every 30 s, and a view re-reads at once when its browser tab becomes visible
  again. *Why:* Northstar §6 — several timers are several freshness clocks that can
  disagree, and a refetch timed from completion cannot pile up behind a slow router.
  *Regression:* no. *Source:* `assets/index.html:2490-2497`, `assets/index.html:2503-2509`,
  `assets/index.html:2652-2657`, `docs/web-console-carbon.md:232-238`,
  `docs/web-console-carbon.md:583-584`, `../northstar/docs/13-console-contracts.md:270-273`,
  `../northstar/docs/13-console-contracts.md:285`.

### Identity and network status

- **SH-C16** The account signing right now is named on screen — its email, else its label,
  with its plan — in the Overview's quota caption and in Settings → Accounts, and as "no
  account" when there is none; a failed read keeps the last name rather than blanking it.
  *Why:* today's header chip says this on every view; Carbon's 48 px header cannot wrap the
  way today's does, so the identity moves to where the quota is. *Regression:* no.
  *Source:* `assets/index.html:400`, `assets/index.html:1137-1138`,
  `docs/web-console-carbon.md:207-209`.
- **SH-C17** The serving account's name leads to Settings → Accounts, where it can be
  switched, edited or removed. *Why:* today's account chip ("Switch, add or remove
  accounts") opens the settings; the way from the identity to its controls stays.
  *Regression:* no. *Source:* `assets/index.html:400`, `assets/index.html:951`.
- **SH-C18** While the router listens beyond loopback, every screen says so and says
  whether a password guards it — guarded in a calm tone, unguarded in the alarm tone — and a
  router on loopback only shows nothing. *Why:* an unguarded network listener is the one
  state worth seeing from anywhere, and a set password is not an alarm. *Regression:* yes —
  check "the password is asked for once, on first boot, from this machine only". *Source:*
  `assets/index.html:401`, `assets/index.html:2408-2417`,
  `scripts/check-dashboard-ui.js:302-321`.
- **SH-C19** The network status's explanation is visible text or a Toggletip, worded for
  the router: on a hosted router "Every caller must give the password"; locally and guarded,
  "Reachable from your network; other devices must give the password"; unguarded, "Reachable
  from your network, but other devices are refused until a password is set". *Why:*
  Carbon's `Tag` drops `title`, and a fact only in a tooltip never reaches a phone.
  *Regression:* yes — check "a hosted router speaks of its deployment, not of this Mac, and a
  local one as it always did". *Source:* `assets/index.html:2413-2416`,
  `scripts/check-dashboard-ui.js:444`, `docs/web-console-carbon.md:323`. *Today:* broken —
  the explanation exists only in the chip's `title`.

### What every panel obeys

- **SH-C20** A missing value renders as an em dash, never as 0, 0 % or $0, and a cost that
  could not be priced is never shown as free. *Why:* Northstar §2 — no data and zero are
  different facts; the dashboard's own `n` and `usd` already say "—". *Regression:* no.
  *Source:* `assets/index.html:669-686`, `assets/index.html:1160`,
  `docs/web-console-carbon.md:446`, `docs/web-console-carbon.md:579-580`,
  `../northstar/docs/13-console-contracts.md:36-47`. *Today:* broken — `k()` turns a missing
  count into "0", and the quota meter reads a missing utilisation as 0 %.
- **SH-C21** Every figure says when it was measured, and where its sources differ in
  freshness it says which one it came from: "checked … ago" from the usage endpoint, "from
  the last request" from response headers. *Why:* header data only moves when traffic
  flows, so a stale reading must not look live (Northstar §1). *Regression:* no.
  *Source:* `assets/index.html:1153-1155`, `assets/index.html:1178`,
  `menubar/src/main.rs:285-292`, `../northstar/docs/13-console-contracts.md:34`.
- **SH-C22** A read that failed and a read that answered "nothing" are two different
  sentences: the first names the failure and its reason, the second says what is empty.
  *Why:* Northstar §2 — an unreachable list must never look like an empty one. *Regression:*
  no. *Source:* `assets/index.html:1157`, `assets/index.html:2449`,
  `../northstar/docs/13-console-contracts.md:40-42`,
  `../northstar/docs/13-console-contracts.md:334-335`.
- **SH-C23** One fetch layer serves every screen and treats as a refusal any non-2xx
  answer, any 200 whose body carries a non-empty `error`, and any `ok: false` (whose sentence
  may be in `reason`); what is shown is the server's sentence: a JSON body's `error`, `reason`
  or `message`, or the one `agent-router: …` line the guard answers with. A refusal that
  carries neither, a page from an ingress in front of the router among them, is said by its
  status, never by its body. *Why:* about a dozen endpoints answer refusals as HTTP 200, and
  the wrapper routes give their reason in `reason`; a layer that throws only on non-2xx reports
  them as successes; and an ingress's HTML while the router is down is not the router's
  sentence. *Regression:* no. *Source:*
  `docs/web-console-carbon.md:224-228`, `assets/index.html:815-820`,
  `src/auth.rs:382-397`, `src/shim.rs:311-339`, `src/hosted.rs:64-68`, `src/guard.rs:187-241`,
  `src/console.rs:283`. *Today:* broken —
  `post()` throws only on a non-2xx status.
- **SH-C24** A refusal appears beside the control that caused it, in the server's words; a
  toast is used only for an action whose control is no longer on screen. *Why:* Northstar §5
  and its `notify.ts` — a generic "failed", or a toast far from the control, loses what to
  fix. *Regression:* no. *Source:* `docs/web-console-carbon.md:536-537`,
  `docs/web-console-carbon.md:581-582`, `../northstar/docs/13-console-contracts.md:253-254`.
- **SH-C25** A poll changes values, never the reader's place: focus, hover, an open menu or
  fold, an expanded row, the scroll position and anything half-typed survive it. *Why:* rows
  are patched by key "so hover, focus and expansion survive", and the scope menus are rebuilt
  only on a scope change "so an open menu is not destroyed by the five-second poll".
  *Regression:* yes — the comments at `assets/index.html:746` and
  `assets/index.html:997-999`. *Source:* `assets/index.html:746-764`,
  `assets/index.html:997-1006`, `assets/index.html:2425-2429`,
  `docs/web-console-carbon.md:112-114`.
- **SH-C26** Text that came from the router, a transcript, a phone or a person — labels,
  emails, paths, error sentences, device names, search snippets — renders as text and never
  as HTML, and search matches are marked by splitting on the router's marker pair, so a stray
  marker can only mis-highlight. *Why:* an `esc()` that missed quotes let model-written text
  add event handlers to a page that approves prompts; text nodes close that class by
  construction. *Regression:* yes — `29c5eb3`; check "a stray marker cannot leave a tag open,
  and snippet text is escaped". *Source:* `assets/index.html:721`,
  `assets/index.html:2559-2563`, `assets/live.html:475-478`,
  `docs/web-console-carbon.md:276-277`, `docs/web-console-carbon.md:435`,
  `scripts/check-dashboard-ui.js:567-575`.
- **SH-C27** No action depends on a native `confirm()`, `prompt()` or `alert()`, and none
  depends on opening a new window without its link copyable beside it; confirmations are
  in-page modals. *Why:* the extension's fallback frame is sandboxed without `allow-modals`
  or `allow-popups`, where a native dialog returns at once and a new window never opens.
  *Regression:* no. *Source:* `vscode/src/dashboard.ts:16-24`,
  `docs/web-console-carbon.md:415-416`, `assets/index.html:587-588`,
  `assets/index.html:1998`. *Today:* broken — Remove account asks through `confirm()`, so it
  does nothing in the fallback frame.
- **SH-C28** No module-level name shadows a browser global the app uses (`history`,
  `location`, `name`, `status`, `top`, `self`, `parent`, `origin`, `screen`, `event`,
  `find`), and lint enforces it. *Why:* a local `history` made `pushState` throw inside
  `openSession`, which silently stopped every card in the list from opening. *Regression:*
  yes — check "a local does not shadow a browser global the page uses" (both scripts), and
  the comment at `assets/live.html:452-453`. *Source:* `assets/live.html:452-454`,
  `assets/live.html:1415-1416`, `scripts/check-dashboard-ui.js:152-158`,
  `scripts/check-live-ui.js:268-274`.
- **SH-C29** An element hidden with the `hidden` attribute stays hidden whatever class it
  carries. *Why:* any class that sets `display` outranks the user agent's rule for `hidden`
  and silently un-hides the element, and Carbon's classes set `display` throughout.
  *Regression:* yes — the comment at `assets/index.html:35-36`. *Source:*
  `assets/index.html:35-37`.

### Tooltips

- **SH-C30** No fact lives only in a tooltip: whatever a tooltip says is also shown as text,
  or reachable by a tap that pins it (`DataTip`) and by keyboard focus. *Why:* a phone has no
  hover and a keyboard cannot reach a `div` (Northstar §7). *Regression:* no. *Source:*
  `assets/index.html:766-806`, `assets/index.html:1166-1168`, `assets/index.html:1390-1395`,
  `assets/index.html:1486-1513`, `assets/index.html:1691`,
  `docs/web-console-carbon.md:104-106`, `docs/web-console-carbon.md:137`,
  `../northstar/docs/13-console-contracts.md:311`. *Today:* broken — the spend split, the p95
  latencies, per-bucket values, reset times, the account id and more exist only on hover,
  mostly on tiles that cannot take focus.
- **SH-C31** A tooltip opens 12 px right of and 14 px below the pointer, flips to the
  pointer's other side rather than cross the viewport's edge, and never comes closer than
  8 px to an edge. *Why:* a tip under the pointer hides what it explains, and a tip off
  screen is no tip. *Regression:* no. *Source:* `assets/index.html:783-789`.
- **SH-C32** A tooltip takes no pointer events, is built from the latest data when it
  opens, follows the pointer within its target, and closes when the pointer leaves the
  target, when focus leaves, and on any scroll. *Why:* a tip that catches the pointer steals
  the hover from what it explains, and one built earlier shows an older poll's numbers.
  *Regression:* no. *Source:* `assets/index.html:336-337`, `assets/index.html:769`,
  `assets/index.html:791-806`.
- **SH-C33** Focusing an element that has a tooltip shows the same tooltip as hovering it,
  anchored under the element. *Why:* the keyboard reaches the same facts as the pointer.
  *Regression:* no. *Source:* `assets/index.html:801-804`.
- **SH-C34** A tooltip pairs each value with its label and a key in its series' colour and
  mark shape — a square for a bar, a line for a line — and a row with no series keeps its
  alignment. *Why:* the key ties the number to the mark it explains. *Regression:* no.
  *Source:* `assets/index.html:338-344`, `assets/index.html:770-777`.

### Theme, colour and motion

- **SH-C35** The theme is a three-way choice — system, light, dark — cycled by one topbar
  button and remembered per browser inside try/catch; "system" follows the operating system
  and keeps following it while the page is open; light is Carbon white and dark is g100.
  *Why:* following the system is what both pages do today and what most readers want; a
  private window means "not remembered", never a failure. *Regression:* no. *Source:*
  `assets/index.html:24-33`, `assets/live.html:6-22`, `../northstar/ui/src/app/theme.ts:66-96`,
  `docs/web-console-carbon.md:173-176`.
- **SH-C36** One theme state is shared by every part of the page, applied as a class on
  `<html>` with a matching `color-scheme`, and set before first paint so a reader who
  prefers dark never sees a light flash. *Why:* Northstar's theme once lived in a `useState`
  per caller, so the switch changed one copy and the page did not change colour at all;
  `color-scheme` makes native controls and scrollbars follow. *Regression:* yes — the comment
  at `../northstar/ui/src/app/theme.ts:24-30`. *Source:*
  `../northstar/ui/src/app/theme.ts:24-56`, `../northstar/ui/index.html:7-18`,
  `assets/live.html:7`.
- **SH-C37** Carbon's theme owns the neutrals, type, spacing, motion and interactive
  chrome; agent-router defines only colours that mean something, one meaning per colour: the
  interactive accent is Carbon's blue, indigo stays the tools colour, and the collisions
  today's pages carry (accent with tools, subagent with tool results, session with Opus,
  project with muted text) are settled. *Why:* a colour that means two things in one view
  means neither. *Regression:* no. *Source:* `docs/web-console-carbon.md:147-148`,
  `docs/web-console-carbon.md:163-176`, `assets/index.html:10-21`.
- **SH-C38** A colour token changes only by re-running the dataviz palette validator in
  both themes against Carbon's own surfaces, never by eye, and every text meets WCAG AA.
  *Why:* the tokens were validated for colour-blind separation, lightness and contrast on
  today's panels, and Carbon g100's `$layer-01` already drops the dark tools colour from
  4.31:1 to 3.65:1. *Regression:* no. *Source:* `assets/index.html:12-14`,
  `docs/web-console-carbon.md:109-111`, `docs/web-console-carbon.md:167-171`. *Today:*
  broken — the dark primary button is about 3:1, white labels inside dark bars 2.56:1, light
  faint text 2.7–3.1:1.
- **SH-C39** The same concept wears the same icon everywhere, from one registry: Folder for
  a project, Terminal for a session, User for an account, Meter for quota, Fork for
  subagents, Flash for cache, WarningAlt for errors, Archive for put away, and Settings for
  settings. *Why:* a reader learns an icon once; today's sprite already gives each concept
  one glyph. *Regression:* no. *Source:* `docs/web-console-carbon.md:140-143`,
  `assets/index.html:369-392`. *Today:* broken — the Settings dialog's title wears the
  account glyph (`assets/index.html:528`).
- **SH-C40** When reduced motion is asked for, nothing transitions, nothing pulses and
  nothing scrolls smoothly, and no state is carried by motion alone. *Why:* both pages honour
  it today — transitions off, the busy and waiting pulses off, back-to-top jumps.
  *Regression:* no. *Source:* `assets/index.html:346`, `assets/index.html:993`,
  `assets/live.html:70-71`, `assets/live.html:112-121`.
- **SH-C41** No text is smaller than 12 px, and the type is IBM Plex Sans and Mono served
  from the binary. *Why:* Carbon's 12 px floor replaces today's 9.5–11.5 px labels, and fonts
  from a CDN would break the no-network promise. *Regression:* no. *Source:*
  `docs/web-console-carbon.md:176-178`, `assets/index.html:91-96`,
  `assets/index.html:1190-1195`, `README.md:111-113`. *Today:* broken — labels at 11 px and
  chart text at 10 px.

### Both widths

- **SH-C42** Every fact and every action exists at both widths; a narrow screen changes the
  layout, never the content. *Why:* Northstar §7 — the same product on a desk and in a hand.
  *Regression:* no. *Source:* `../northstar/docs/13-console-contracts.md:294-297`,
  `docs/web-console-carbon.md:424-426`, `assets/index.html:347-359`. *Today:* broken — at
  700 px and below, a bar row's path and a request row's time to first token are
  `display: none`.
- **SH-C43** The console has one breakpoint, Carbon's 41.98 rem, read by the stylesheets and
  by script through one module. *Why:* a 671 px query in Northstar's script disagreed with
  Carbon's 41.98 rem at fractional zoom, and today's page mixes a 700 px media query with an
  `innerWidth < 700` test. *Regression:* yes — Northstar `16d666d`. *Source:*
  `../northstar/ui/src/lib/viewport.ts:3-9`, `docs/web-console-carbon.md:540`,
  `docs/web-console-carbon.md:613-614`, `assets/index.html:347`, `assets/index.html:1544`.
- **SH-C44** Nothing scrolls sideways at 360 px: transcripts, tool output, paths and URLs
  wrap anywhere, and no grid or row has a minimum wider than the screen. *Why:* one unbroken
  URL widened a 390 px Northstar page to 1,119 px. *Regression:* yes — Northstar `4b65e8b`.
  *Source:* `docs/web-console-carbon.md:101-103`, `docs/web-console-carbon.md:424-426`,
  `docs/web-console-carbon.md:606-607`, `assets/index.html:157`, `assets/index.html:222`,
  `assets/index.html:247`. *Today:* broken — the dashboard scrolls sideways by 143, 128 and
  89 px at 360, 375 and 414 px.
- **SH-C45** On a coarse pointer every target is at least 48 px and every field's text at
  least 16 px, sized by pointer type rather than by width, and the copied `FormModal` and
  `ReasonModal` get the 16 px their comments promise. *Why:* iOS zooms into a field under
  16 px and never zooms back out, and a fingertip misses a 26 px target. *Regression:* yes —
  Northstar `dbf9481`. *Source:* `../northstar/docs/13-console-contracts.md:312-313`,
  `docs/web-console-carbon.md:107-108`, `docs/web-console-carbon.md:326-327`,
  `docs/web-console-carbon.md:542-544`, `docs/web-console-carbon.md:611-612`,
  `assets/index.html:323`, `assets/index.html:330-331`. *Today:* broken — fields at 12–15 px
  and targets of 26–37 px.
- **SH-C46** Whatever appears on hover appears outright where there is no hover. *Why:*
  Northstar §7. *Regression:* no. *Source:* `../northstar/docs/13-console-contracts.md:311`,
  `../northstar/ui/src/lib/viewport.ts:32-35`, `assets/live.html:362-364`. *Today:* broken —
  a session tab's put-away glyph is invisible until hovered, except on the active tab.
- **SH-C47** The one document carries the live view's viewport settings —
  `viewport-fit=cover` and `interactive-widget=resizes-content` — so Sessions keeps its
  100 dvh column, its safe-area insets and its visual-viewport keyboard pin inside the
  shell's content area. *Why:* one document now serves every route, and on iOS the URL bar
  and the keyboard both move the composer without them. *Regression:* yes — the comments at
  `assets/live.html:27-31` and `assets/live.html:1662-1664`. *Source:* `assets/live.html:3`,
  `assets/live.html:25-37`, `assets/live.html:1662-1679`, `docs/web-console-carbon.md:210-212`.
- **SH-C48** On the screens whose page scrolls (the Overview and Settings), a "Back to top"
  button with that accessible name appears once the page is 600 px down and returns to the
  top, smoothly unless reduced motion is asked for; Sessions, whose panes scroll themselves
  and whose composer holds the bottom edge, has none. *Why:* the Overview is long, and
  today's button belongs to the dashboard only. *Regression:* no. *Source:*
  `assets/index.html:292-296`, `assets/index.html:646-648`, `assets/index.html:991-994`,
  `assets/live.html:73-77`.

### Keyboard, URL and the palette

- **SH-C49** `/` focuses and selects the Overview's search field, except while focus is in a
  text field, a select or anything editable, or with ⌘, Ctrl or Alt held, so a slash can
  always be typed; the field shows a `/` hint. *Why:* today's shortcut, written never to eat
  a typed slash. *Regression:* no. *Source:* `assets/index.html:420-422`,
  `assets/index.html:2573-2578`.
- **SH-C50** ⌘K (Ctrl-K off a Mac) opens the palette from any screen, even from inside a
  field, and the topbar's search action opens the same palette, which is the whole of it on
  a phone. *Why:* Northstar §8 makes the palette primary navigation. *Regression:* no.
  *Source:* `../northstar/ui/src/screens/Palette/shortcut.ts:16-20`,
  `../northstar/ui/src/app/Shell.tsx:108-113`, `docs/web-console-carbon.md:201-204`,
  `../northstar/docs/13-console-contracts.md:330-331`.
- **SH-C51** The palette is a passive modal near the top on a desk and a full-height sheet
  under the topbar on a phone; it searches the router's conversation index (`/api/search`),
  jumps to a project or a session, and runs commands behind `>` — switch account, set a
  session's model, effort or mode, archive, theme — each through the same request and the
  same refusal handling as the control on its own screen. *Why:* one way to do a thing,
  whichever door it is reached by. *Regression:* no. *Source:*
  `docs/web-console-carbon.md:201-204`, `docs/web-console-carbon.md:549-550`.
- **SH-C52** A search answers the latest keystroke: input is debounced, and a reply to an
  earlier query that arrives after a later one is dropped. *Why:* "a later keystroke has
  already answered". *Regression:* no. *Source:* `assets/index.html:2554-2557`,
  `assets/index.html:2572`, `assets/index.html:2580-2602`.
- **SH-C53** The URL is the view: scope, time range, request filters, Find and the open
  session live in it, so a view is a link that pastes and reloads as itself; typing replaces
  the history entry, while a change of scope or screen pushes one, so Back undoes it. *Why:*
  Northstar §8, and the README's promise that "a link keeps its scope". *Regression:* no.
  *Source:* `docs/web-console-carbon.md:218-220`,
  `../northstar/docs/13-console-contracts.md:325-327`, `README.md:109-111`,
  `assets/index.html:914-923`. *Today:* broken — the dashboard replaces the history entry on a
  scope change, so Back leaves the page instead of un-scoping it.
- **SH-C54** A draft belongs to where it was typed: moving to another screen, Settings tab,
  account or session never carries a half-typed value along and never sends it there. *Why:*
  a route-parameter change keeps a screen mounted, and Northstar saved drafts onto the next
  project that way. *Regression:* yes — Northstar `940a783`. *Source:*
  `docs/web-console-carbon.md:243-245`, `docs/web-console-carbon.md:598-600`.

### Where the console is opened

- **SH-C55** The console works wherever it is opened: in VS Code's Simple Browser (a framed
  page), in the extension's fallback webview frame, in the Integrated Browser of newer VS
  Code, in a system browser, and in Safari from the iPhone app — framed or not, and with
  browser storage that may be unavailable. *Why:* the extension opens `/` and `/live` through
  Simple Browser and falls back to a webview frame, with the system browser a click away, and
  the iPhone's settings open the dashboard in Safari. *Regression:* no. *Source:*
  `vscode/src/dashboard.ts:1-29`, `vscode/src/extension.ts:18-19`, `ARCHITECTURE.md:374-376`,
  `ios/AgentRouter/Sources/Views/RouterSettingsView.swift:146-148`.


## Overview

### Layout and identity

- **OV-C1** The Overview keeps every section the dashboard shows, in its order: the scope bar;
  Search the transcripts; Quota headroom (meters, history, burn-down); All accounts; Totals in
  scope; Activity over time; What the model read beside What fills the context window; By model
  beside Main thread vs subagents; Accounts; Projects; Sessions; Tool output the model read beside
  Tool calls made; Requests. *Why:* the Overview's brief is to keep everything the dashboard shows,
  in the reading order its readers know. *Regression:* no. *Source:* `assets/index.html:405-511`.
- **OV-C2** Every section heading carries a caption saying what the section measures or how to use
  it — "the account that is serving right now", "cache vs. paid in full", "estimate · by character
  share", "prompt tokens read", "share of prompt tokens", "est. tokens", "by the model" — as visible
  text beside the heading, never in a tooltip. *Why:* an explanation belongs next to what it
  explains (the user's standing preference), and Carbon's `Tag` drops `title`. *Regression:* no.
  *Source:* `assets/index.html:417`, `assets/index.html:439`, `assets/index.html:461-501`.
- **OV-C3** Every control is visible at rest — the time range, Clear scope, the search kinds, the
  request filters, Show more — and none sits in an overflow menu or behind a disclosure. *Why:* the
  user's standing preference: controls visible, not behind disclosure. *Regression:* no. *Source:*
  `assets/index.html:405-429`, `assets/index.html:506-511`.
- **OV-C4** One icon per concept, the same wherever the concept appears: project (folder, in the
  project colour), session (terminal, in the session colour), account (person), quota (gauge),
  subagents (fork), main thread, cache (flash), errors (warning), tools (wrench), system prompt,
  conversation, tool results, context window, model, activity, requests, search, git branch, slow
  (clock) and scope (funnel); in the Carbon app they come from the one `concepts.ts` registry.
  *Why:* the user's standing preference, and the plan's registry (Folder, Terminal, User, Meter,
  Fork, Flash, WarningAlt). *Regression:* no. *Source:* `assets/index.html:363-393`,
  `assets/index.html:659-664`, `assets/index.html:1016-1025`.
- **OV-C5** Errors wear one mark everywhere: the error icon in the error colour beside the count, in
  model, project and session rows as in tool rows and request rows. *Why:* the same concept wears
  the same icon. *Regression:* no. *Source:* `assets/index.html:1637`, `assets/index.html:1715`,
  `assets/index.html:1759`, `assets/index.html:869-870`, `assets/index.html:1897`. *Today:* broken —
  models, projects and sessions write a text "⚠ N" inside the grey meta line, while tools and
  requests use the error icon in red.
- **OV-C6** The error icon means an error and nothing else: the "not yet attributed" row, which
  counts requests waiting for their transcript rather than failures, wears a mark of its own. *Why:*
  an icon that means two things makes the reader stop to work out which. *Regression:* no. *Source:*
  `assets/index.html:1731-1732`. *Today:* broken — the row wears the error triangle, greyed.
- **OV-C7** The colours that carry meaning are kept as tokens over Carbon's theme: the four context
  components (tools, system prompt, conversation, tool results), the token-magnitude ramp (one hue,
  more is darker), identity colours for project, session and subagent, the four model families, and
  ok, warn and bad with one text-safe and one fill-safe token each; Carbon owns the neutrals and the
  chrome. *Why:* "hue is reserved for identity"; agent-router defines only colours that mean
  something, and Carbon's support colours fail 4.5:1 as text. *Regression:* no. *Source:*
  `assets/index.html:9-33`.
- **OV-C8** No two meanings share a colour: tools are not the accent, subagents are not tool results
  (both appear in one request's detail), a session is not Opus, and a project is not muted text.
  *Why:* a colour that means two things means neither; the plan lists these four collisions to
  settle. *Regression:* no. *Source:* `assets/index.html:10-21`. *Today:* broken — the light accent
  equals `--c1`, `--sub` is `var(--c4)`, `--sess` equals `--m-opus`, and `--proj` equals `--muted`.
- **OV-C9** Every meaningful colour is checked with the dataviz palette validator — colour-blind
  separation, lightness band, contrast — in both themes and against the surfaces it is drawn on,
  Carbon's white and g100 layers included, and the validator is re-run rather than a value adjusted
  by eye. *Why:* the stylesheet says so, and on g100's `$layer-01` the dark `--c1` falls from 4.31:1
  to 3.65:1. *Regression:* no. *Source:* `assets/index.html:12-14`, `assets/index.html:24-33`.
- **OV-C10** A model's family is read from its name — opus, sonnet, haiku, anything else is other —
  and wears that family's colour on every swatch, bar, chip and chart, other being a hollow ring
  rather than a fourth fill. *Why:* the family is the identity the activity chart and every model
  mark share, and the ring keeps "other" from reading as one more hue. *Regression:* no. *Source:*
  `assets/index.html:21`, `assets/index.html:234-235`, `assets/index.html:665-666`,
  `assets/index.html:712`, `assets/index.html:742`. *Phones:* `testRangesAndFormats`.
- **OV-C11** Whatever the page turns off is not displayed and takes no space, an SVG included,
  whatever classes style it. *Why:* "Any class that sets `display` outranks the user-agent rule for
  the hidden attribute, which silently un-hides elements", and in Chrome `hidden` does not hide an
  `<svg>`, which then takes a 300×150 box. *Regression:* yes — the comments at
  `assets/index.html:35-36` and `assets/index.html:363-365` record both. *Source:*
  `assets/index.html:35-37`, `assets/index.html:363-367`.

### Scope

- **OV-C12** The scope is three nested levels, account › project › session, plus a time range, shown
  as one breadcrumb: the scope icon, then a crumb per level with its concept's icon and colour,
  reading "All accounts", "All projects" and "All sessions" until a level is set and in the strong
  style once it is. *Why:* where the page is narrowed and how to change it are one control.
  *Regression:* no. *Source:* `assets/index.html:405-415`, `assets/index.html:1000-1025`.
- **OV-C13** A crumb names its level the way the lists do: an account by its label (its router id
  until the summary knows it), a project by the last part of its path with the whole path one press
  away, a session by its first eight characters followed by its git branch when known. *Why:* the
  crumb and the row it came from must read as the same thing. *Regression:* no. *Source:*
  `assets/index.html:1003`, `assets/index.html:1014-1025`.
- **OV-C14** Every crumb is a picker, so a scope can be chosen at the top of the page instead of
  hunting for its row further down. *Why:* the comment above `renderCrumbs`. *Regression:* no.
  *Source:* `assets/index.html:997-1013`.
- **OV-C15** The five-second poll never closes, rebuilds or moves an open picker; the crumbs change
  only when the scope, or the scoped session's branch, does. *Why:* "Rebuilt only when the scope
  changes, so an open menu is not destroyed by the five-second poll." *Regression:* no. *Source:*
  `assets/index.html:997-1006`.
- **OV-C16** A picker lists "All …" first, then one row per account, project or session, each with
  its name, a second line and a meta of tokens · requests · cost, the current choice marked; the
  name never shrinks, and a project's path gives up the space and ellipsises from the left, keeping
  its meaningful tail. A session's second line is its branch, its project (unless one is scoped) and
  its start; an account's is its router id and "active". *Why:* the comment on `.menu .mname`.
  *Regression:* no. *Source:* `assets/index.html:88-96`, `assets/index.html:1039-1074`,
  `assets/index.html:1086-1093`.
- **OV-C17** An account is shown with its router-side id beside its label, in the picker and in the
  Accounts list. *Why:* "Two rows can carry the same identity (the discovered Keychain login and a
  token for the same person), so the router-side id is what tells them apart." *Regression:* no.
  *Source:* `assets/index.html:1047-1050`, `assets/index.html:1686`.
- **OV-C18** A picker of more than eight rows opens with a filter that matches the name and the
  second line, ignoring case, says "Nothing matches." when nothing does, and names what it filters
  in its placeholder. *Why:* a long list needs a way in, and a filter that names the wrong list
  misleads. *Regression:* no. *Source:* `assets/index.html:1083-1100`. *Today:* broken — the account
  picker's filter reads "Filter sessions…".
- **OV-C19** Opening a picker puts the focus in its filter, else on the current row, else on the
  first; Up and Down move through the rows and wrap; Escape closes it and gives the focus back to
  its crumb; a press outside it, a window resize or a second press on its crumb closes it. *Why:* a
  picker is used from the keyboard as much as from the pointer. *Regression:* no. *Source:*
  `assets/index.html:1075-1080`, `assets/index.html:1110-1126`.
- **OV-C20** A picker opens under its crumb, pulled back inside the window with an 8 px margin, at
  most 60 % of the window (420 px) tall and scrolling within. *Why:* it is measured and then placed,
  so the window's edge never cuts it off. *Regression:* no. *Source:* `assets/index.html:79-81`,
  `assets/index.html:1105-1109`.
- **OV-C21** A picker is a filter field above a listbox of options, not a `role=menu` with a text
  field inside it. *Why:* a menu cannot hold an input, so assistive technology announces it wrongly;
  the plan's `ScopePicker` uses the listbox pattern. *Regression:* no. *Source:*
  `assets/index.html:650`, `assets/index.html:1087`, `assets/index.html:1097`. *Today:* broken —
  `#menu` is `role=menu` and holds an `<input>` beside its `menuitem` buttons.
- **OV-C22** Choosing a level clears the levels below it and keeps those above: an account clears
  the project and session, a project clears the session, and a session also sets its own project.
  *Why:* a narrower scope chosen under another parent would select nothing. *Regression:* no.
  *Source:* `assets/index.html:1052`, `assets/index.html:1062`, `assets/index.html:1072`,
  `assets/index.html:1681`, `assets/index.html:1709`, `assets/index.html:1747`.
- **OV-C23** Pressing a list's selected row again clears that level, and leaving a level does the
  same from its picker as from its list. *Why:* one action, one outcome. *Regression:* no. *Source:*
  `assets/index.html:1044-1045`, `assets/index.html:1681`, `assets/index.html:1709`,
  `assets/index.html:1747`. *Today:* broken — "All accounts" in the picker also clears the project
  and the session, while pressing the selected account in the Accounts list keeps them.
- **OV-C24** Scoping to a session from anywhere — its picker, the Sessions list, a search's session
  row or a search hit — gives the same scope: that session and its project. *Why:* the crumb and the
  tiles must name the project of the session they describe. *Regression:* no. *Source:*
  `assets/index.html:1072`, `assets/index.html:1747`, `assets/index.html:2618`,
  `assets/index.html:2647`. *Today:* broken — the search's rows and hits set the project to none, so
  the crumb reads "All projects" and the Requests tile "1 session · (unresolved)" for a session that
  has a project.
- **OV-C25** "Clear scope" shows exactly when an account, project or session is set; it clears all
  three and keeps the time range. *Why:* the range is a separate choice from what is narrowed.
  *Regression:* no. *Source:* `assets/index.html:414`, `assets/index.html:940`,
  `assets/index.html:1002`.
- **OV-C26** The time range is one of all, 1h, 24h, 7d and 30d, all by default, as an always-visible
  single-choice switcher named "Time range"; a relative range is measured back from the moment of
  each read, so it slides with the clock. *Why:* each read asks for its lower bound afresh.
  *Regression:* no. *Source:* `assets/index.html:407-413`, `assets/index.html:658`,
  `assets/index.html:912-913`, `assets/index.html:2504`. *Phones:* `testRangesAndFormats`.
- **OV-C27** The time range narrows every figure on the Overview at once — totals, activity and
  quota history, read and context, models, threads, the account split, the lists, the requests, the
  all-accounts usage and the search — except the quota meters, the burn-down and the 30-day pace,
  which keep horizons of their own and show them. *Why:* "A time range the dashboard applies to
  everything on the page at once." *Regression:* no. *Source:* `assets/index.html:2510-2519`,
  `assets/index.html:2591-2592`, `src/api.rs:56-57`, `src/api.rs:1004-1008`.
- **OV-C28** A range named in a link that is not one of the five reads as all, never as a property
  every object has (`constructor`, `__proto__`). *Why:* the address is input anyone can type.
  *Regression:* no. *Source:* `assets/index.html:912-913`, `assets/index.html:928-929`.
- **OV-C29** The scope, the range and the open request rows are in the address, so a scoped view is a link. An address in the old dashboard's spelling, `#a=&p=&s=&r=&o=`, opens the Overview unscoped: it is neither read nor rewritten. *Why:* a link to a scoped view has to survive a reload; the clean cut (decision J) keeps no compatibility code for the old page's addresses. *Regression:* no. *Source:* `assets/index.html:908-925` (the old spelling); plan, decision J.
- **OV-C30** A scope or range change pushes a history entry, so Back un-scopes the page. *Why:*
  Northstar's rule, adopted by the plan; the dashboard replaces the entry today, so Back leaves the
  page. *Regression:* no. *Source:* `assets/index.html:914-923`.
- **OV-C31** The request filters — kind, model, stop reason — live in the address with the scope.
  *Why:* the plan puts scope, range and request filters in the query string; today they live in
  memory and a reload drops them. *Regression:* no. *Source:* `assets/index.html:655-656`,
  `assets/index.html:914-923`.
- **OV-C32** An edited or followed address re-scopes the page without a reload. *Why:* the address
  is the scope. *Regression:* no. *Source:* `assets/index.html:2660`.
- **OV-C33** A scope change reads at once rather than at the next tick, closes any open picker
  without moving the focus, and returns the request list to its first twenty rows. *Why:* what was
  chosen should show at once, and from the top. *Regression:* no. *Source:*
  `assets/index.html:932-938`.
- **OV-C34** The session picker says when its list stops at the 200 most recent sessions, as the
  Sessions list does. *Why:* truncation is loud (Northstar §1), and a picker states its population
  when it opens (Northstar §8). *Regression:* no. *Source:* `assets/index.html:1065-1067`,
  `assets/index.html:1745`, `src/api.rs:608`. *Today:* broken — its "All sessions" row reads "200"
  with no word that the list was cut.
- **OV-C35** The scope bar stays in view at the top while the page scrolls. *Why:* the scope
  qualifies every figure below it. *Regression:* no. *Source:* `assets/index.html:67-68`.

### Search

- **OV-C36** The search box is labelled "Search the transcripts", invites "a word, a phrase, an
  identifier…", does no spell-checking or autocompletion, and while empty its heading's caption
  reads "every message, tool call and result ever captured". *Why:* identifiers are typed exactly as
  they appear. *Regression:* no. *Source:* `assets/index.html:417-421`,
  `assets/index.html:2583-2586`.
- **OV-C37** "/" focuses the search box and selects its text from anywhere on the page, except while
  typing in a field or with a modifier held, and a visible "/" hint sits beside the box. *Why:* the
  shortcut is worth having only if it can be found and never steals a keystroke. *Regression:* no.
  *Source:* `assets/index.html:422`, `assets/index.html:2573-2578`.
- **OV-C38** The five kinds — all, messages, tool calls, results, thinking — are an always-visible
  single-choice switcher that never collapses into a dropdown, all being the default. *Why:* the
  plan, and controls stay visible. *Regression:* no. *Source:* `assets/index.html:423-429`,
  `assets/index.html:2565`.
- **OV-C39** Typing searches 160 ms after the last keystroke; changing the kind searches at once.
  *Why:* one request per pause, not one per letter. *Regression:* no. *Source:*
  `assets/index.html:2567-2572`.
- **OV-C40** A blank query puts the results away and restores the idle caption rather than searching
  for nothing. *Why:* check "an empty query puts the panel away rather than searching for nothing".
  *Regression:* no. *Source:* `assets/index.html:2581-2587`,
  `scripts/check-dashboard-ui.js:536-540`.
- **OV-C41** A reply that arrives after a newer query was sent is dropped, error or not: an older,
  slower search never overwrites a newer one. *Why:* "a later keystroke has already answered".
  *Regression:* no. *Source:* `assets/index.html:2557`, `assets/index.html:2581`,
  `assets/index.html:2594`, `assets/index.html:2602`.
- **OV-C42** The search is not part of the five-second poll and the router does not cache it: it
  answers the keyboard, not the clock. *Why:* the comment at its head, and the router memoises fixed
  questions only. *Regression:* no. *Source:* `assets/index.html:2554`, `src/api.rs:626-631`.
- **OV-C43** The typed text goes to the router verbatim, trimmed, so phrase, prefix, NEAR and OR
  searches work; ordinary text that syntax rejects is retried by the router as an AND of quoted
  words, so only a store failure ever surfaces. *Why:* `src/api.rs:647-656`. *Regression:* no.
  *Source:* `assets/index.html:2581`, `assets/index.html:2591`, `src/api.rs:647-663`.
- **OV-C44** The search follows the page's project, session and time range but not its account,
  which the search API does not take, and says so — idle, with hits and without — whenever one of
  those three narrows it. *Why:* the reader must know whether the scope hid a match. *Regression:*
  no. *Source:* `assets/index.html:2591-2592`, `assets/index.html:2634`, `src/api.rs:612-624`.
  *Today:* broken — only the no-match message says "in the current scope"; the idle caption claims
  "every message, tool call and result ever captured" under any range.
- **OV-C45** A failed search says so where the results go, in the router's own words: "Search
  failed: <message>". *Why:* a refusal renders in the server's own words (Northstar §5).
  *Regression:* no. *Source:* `assets/index.html:809-814`, `assets/index.html:2593-2600`,
  `src/api.rs:122-130`. *Today:* broken — the message is the reply's body as text, so it reads
  `Search failed: {"error":"…"}`.
- **OV-C46** Results come in two parts: "Sessions that mention it — click one to scope the page", a
  bar row per session sized by its matching places, with its project path, "N messages · <last
  seen>" and its full id one press away; then "Best matches — N shown", best first. *Why:* which
  conversations talk about it, then where; check "a search renders its sessions and its hits".
  *Regression:* no. *Source:* `assets/index.html:2610-2632`, `src/api.rs:673-717`,
  `scripts/check-dashboard-ui.js:525-534`.
- **OV-C47** The caption states what came back and how long it took — "N sessions · X ms" or
  "nothing matched · X ms" — and says so when the router's 200-session cap was reached. *Why:*
  truncation is loud. *Regression:* no. *Source:* `assets/index.html:2603-2608`, `src/api.rs:715`.
  *Today:* broken — at the cap it reads "200 sessions", with no word that more matched.
- **OV-C48** A hit is a button naming its kind, its role, its project, its short session id and its
  time, over a snippet of at most three lines; pressing it scopes the page to that session. *Why:* a
  hit is a way into its conversation. *Regression:* no. *Source:* `assets/index.html:195-205`,
  `assets/index.html:2637-2649`.
- **OV-C49** Matches are marked by splitting the snippet on its ‹ › pair, never by inserting markup:
  a stray marker in the source can only mis-highlight, never leave an element open, and the
  snippet's own text stays text. *Why:* the comment on `markup`; check "a stray marker cannot leave
  a tag open, and snippet text is escaped". *Regression:* no. *Source:*
  `assets/index.html:2559-2563`, `assets/index.html:2646`, `scripts/check-dashboard-ui.js:567-575`.
  *Phones:* `testSearchResultsDecode`.
- **OV-C50** Each kind a hit can carry has the reader's word for it, the same on every client:
  message, tool call, tool result, thinking, system prompt. *Why:* "the wire's vocabulary is not the
  reader's", and the router indexes system-prompt blocks too (`src/analyze.rs:315`). *Regression:*
  no. *Source:* `assets/index.html:2555`, `assets/index.html:2640`,
  `ios/AgentRouter/Sources/API/SearchModels.swift:44-54`. *Phones:* `testSearchResultsDecode`.
  *Today:* broken — the page has no word for `system` and prints the wire's.
- **OV-C51** A hit's identity is its session, block and position together, never the block alone.
  *Why:* "The same block can appear at several places", and a list keyed on duplicates drops rows.
  *Regression:* no. *Source:* `assets/index.html:2637-2649`,
  `ios/AgentRouter/Sources/API/SearchModels.swift:37-39`. *Phones:* `testSearchResultsDecode`.
- **OV-C52** A hit or session whose project is unknown still renders, and names its project as the
  lists do, "(unresolved)". *Why:* the join behind it is a LEFT JOIN, so a request whose transcript
  never arrived has no project, and "(unresolved)" is the dashboard's label for that
  (`src/api.rs:67-68`). *Regression:* no. *Source:* `assets/index.html:715`,
  `assets/index.html:2628`, `assets/index.html:2642`. *Phones:*
  `testSearchToleratesMissingProjectAndLabel`. *Today:* broken — the phone prints "—" where the page
  prints "(unresolved)".
- **OV-C53** The search's session rows mark the session the page is scoped to as soon as it changes.
  *Why:* a pressed row that is not the scope misleads. *Regression:* no. *Source:*
  `assets/index.html:2614-2629`. *Today:* broken — the mark is set only when a search runs, so
  pressing a session row scopes the page but leaves that row unmarked until the next keystroke.

### Quota

- **OV-C54** The quota panel always describes the account serving now, whatever account, project or
  session the page is scoped to, and its caption names that account. *Why:* quota belongs to the
  account, not the project (`src/api.rs:302-305`); the header chip that names the account today does
  not fit Carbon's 48 px header. *Regression:* yes — 54d3c94: the page pooled every account while
  the quota showed only the serving one, and the two disagreed. *Source:* `assets/index.html:439`,
  `assets/index.html:2531-2535`, `src/api.rs:347-362`.
- **OV-C55** The panel prefers the live usage reading, which the router polls from Anthropic and
  which stays current on an idle machine, and falls back to the last response's rate-limit headers,
  which move only when traffic flows. *Why:* the comment at the head of `renderQuota`. *Regression:*
  no. *Source:* `assets/index.html:1144-1156`, `src/quota.rs:184-216`.
- **OV-C56** The panel says where its numbers came from and how old they are: "checked <ago>" for
  the live reading, "from the last request <ago>" for headers. *Why:* "a response header is only as
  fresh as the last request, while the usage endpoint is current even on an idle machine."
  *Regression:* no. *Source:* `assets/index.html:1152-1155`, `assets/index.html:1178`.
- **OV-C57** One meter per window, 5-hour and 7-day, reads "N% used · resets in …" over a bar capped
  at 100 %, amber from 70 % and red from 90 %; it is a meter to assistive technology and no poll
  re-announces it. *Why:* the plan's `UsageMeter` (`role=meter`), since Carbon's `ProgressBar` would
  re-announce every five seconds. *Regression:* no. *Source:* `assets/index.html:105-109`,
  `assets/index.html:1159-1173`.
- **OV-C58** A window with no reading shows "—", never "0% used". *Why:* missing is not zero
  (Northstar §1), and the plan lists this among the wrong figures. *Regression:* no. *Source:*
  `assets/index.html:1148-1149`, `assets/index.html:1160`, `src/quota.rs:344-345`. *Today:* broken —
  a missing utilisation, or a usage reply without a 7-day window, is read as zero and drawn as "0%
  used".
- **OV-C59** The status tag's colour and its word come from the same field, and the error icon
  appears only on a red tag. *Why:* a red tag that says "normal" contradicts itself; the plan lists
  it among the wrong figures. *Regression:* no. *Source:* `assets/index.html:1174-1177`. *Today:*
  broken — the colour also reads the last header's `rl_status` while the word reads the live
  severity, so a stale `allowed_warning` paints a tag that says "normal" red.
- **OV-C60** Under the meters, the quota history draws the 5-hour and 7-day percentages as two lines
  on the activity chart's time axis — one point per bucket, the worst reading in it, capped at 100 %
  — with rules at 100 %, 90 % and 0, the first and last bucket's times, the latest point marked on
  each line and the legend "percent used, over the window charted below"; pointing at it gives the
  nearest point's time and both percentages. *Why:* "two lines over the same time axis as the
  activity chart", and "The worst reading in a bucket is the one that matters for headroom."
  *Regression:* no. *Source:* `assets/index.html:1180-1220`, `src/api.rs:517-530`.
- **OV-C61** A history point with no reading for a window is left off that window's line and reads
  "—" in its details, never 0 %. *Why:* missing is not zero. *Regression:* no. *Source:*
  `assets/index.html:1189`, `assets/index.html:1198`, `assets/index.html:1216`. *Today:* broken —
  the details read "0%" for the missing window, and when the latest point lacks one, that line's end
  marker sits on the 0 line.
- **OV-C62** Each window keeps one colour on every meter, line and chart — the 5-hour the accent,
  the 7-day slate — and amber and red replace it only past their thresholds. *Why:* the same thing
  wears the same colour. *Regression:* no. *Source:* `assets/index.html:1161`,
  `assets/index.html:1196-1197`, `assets/index.html:1325-1326`, `assets/index.html:1431-1432`.
  *Today:* broken — the quota panel draws its 7-day meter in the accent like the 5-hour one, while
  its history line, its burn-down and the account cards draw the 7-day window in slate.
- **OV-C63** The times behind each window — the exact reset and, on the burn-down, when the window
  opened and the usage now — are readable without hovering, on a phone and from the keyboard. *Why:*
  the plan lists reset times among the facts a phone cannot reach. *Regression:* no. *Source:*
  `assets/index.html:1166-1168`, `assets/index.html:1343-1351`. *Today:* broken — the meters show
  only "resets in …", and the rest is in tooltips.
- **OV-C64** A quota or forecast read that fails never blanks the page: the panel keeps its last
  good reading, with its age, and the rest of the page draws as usual. *Why:* "Optional: present
  only on builds that carry it, and a failure must not blank the page." *Regression:* no. *Source:*
  `assets/index.html:2517-2519`, `assets/index.html:2529-2530`.
- **OV-C65** A reading kept from an earlier read is shown only for the account and the range it was
  read for. *Why:* a kept value is honest only about what it measured. *Regression:* no. *Source:*
  `assets/index.html:2529-2534`. *Today:* broken — the last good forecast and live quota are reused
  whatever the range or the serving account has become, so after a range change a failed forecast
  read leaves the previous range's spend under "in the selected range".

### Forecast

- **OV-C66** The forecast answers one question per window — does the way you are working reach 100 %
  before the reset Anthropic already gave? — and measures burn per hour of work — a pause between
  requests counts as work only up to five minutes, the rest being absence — never per hour on the
  clock. *Why:* fitted against the clock, a 31-hour idle gap made the reading wrong by a day and a
  half. *Regression:* yes — the 7-day window that reset on 2026-09-11, pinned by
  `an_overnight_gap_does_not_change_the_burn_rate` in `src/forecast.rs`. *Source:*
  `assets/index.html:1223-1228`, `src/forecast.rs:7-21`, `src/forecast.rs:42-44`.
- **OV-C67** Work left is stated first, in hours of work, the measured number; a calendar time
  follows it, because a date is derived through a duty cycle and is the softer of the two. *Why:*
  "Burn is a measurement; duty is a forecast, and the two do not deserve the same typeface."
  *Regression:* no. *Source:* `assets/index.html:1243-1255`, `src/forecast.rs:17-26`.
- **OV-C68** The verdict has three states: red only for crosses, where even a quiet week exhausts
  the window; amber for tight, where the band straddles the reset; plain for resets first, amber
  when it ends at 90 % or more. *Why:* "a margin that lands inside the spread of your own working
  days is a coin flip, not a finding". *Regression:* yes — 5b2c120: on 2026-09-14 a 7-day window at
  49 %, crossing 2.2 h before a reset 94.7 h away, was rendered red; pinned by check "every window
  state produces a line, and only the real trouble is red" and by
  `a_crossing_inside_the_spread_of_your_own_days_is_tight_not_critical`. *Source:*
  `assets/index.html:1233-1256`, `scripts/check-dashboard-ui.js:210-228`, `src/forecast.rs:119-133`.
  *Phones:* `testRunsOutBeforeTheResetIsRed`, `testAStraddlingBandIsAmberAndSaysSo`,
  `testResetsFirstIsCalm`, `testNearlyFullAtResetWarns`.
- **OV-C69** The words are the same on every client, a phone free to capitalise the first: crosses
  "H of work left · runs out in …, D before it resets"; tight "H of work left · too close to call —
  on this pace it runs out D before reset, on a quieter week it ends ≈N%"; resets first "H of work
  left · resets first, ending ≈N%". *Why:* "the phone and the Mac never disagree about the same
  numbers" (`Projection.swift`); check "a straddling band is amber and says so, and the band is
  disclosed". *Regression:* no. *Source:* `assets/index.html:1243-1255`,
  `scripts/check-dashboard-ui.js:230-240`. *Phones:* `testRunsOutBeforeTheResetIsRed`,
  `testAStraddlingBandIsAmberAndSaysSo`, `testResetsFirstIsCalm`.
- **OV-C70** A tight verdict whose central estimate does not cross never prints a crossing it was
  not given; it stays amber and says too close to call. *Why:* tight means only the busy edge
  crosses, so the central `exhausts_at` may be absent (`src/forecast.rs:312-318`). *Regression:* no.
  *Source:* `assets/index.html:1245-1248`. *Today:* broken — the page subtracts nothing from the
  reset and prints "runs out ~20000d before reset"; the phone avoids it by falling back to the
  resets-first sentence.
- **OV-C71** A router that sends no verdict, from before the band, is read by the old boolean — a
  crossing time means crosses, none means resets first — and the page never goes quiet for it.
  *Why:* "fall back to the old boolean there rather than going quiet"; check "a router that predates
  the band still renders as it always did". *Regression:* no. *Source:*
  `assets/index.html:1237-1238`, `assets/index.html:1244`, `assets/index.html:1308-1309`,
  `scripts/check-dashboard-ui.js:242-254`. *Phones:* `testAnOlderRouterStillRendersAsItAlwaysDid`. *Under decision J:* the console ships inside the router it reads, so the older-router case here never reaches it; the rest of the rule stands, and the phones still meet older routers.
- **OV-C72** A window whose reset has passed reads "window rolled over <ago> · nothing seen since",
  plain, with no basis, no meter value and no burn-down: an old utilisation is never shown as
  current. *Why:* "reporting the old utilisation as current would be a lie with a number on it"
  (`src/api.rs:1085-1087`). *Regression:* no. *Source:* `assets/index.html:1241`,
  `assets/index.html:1260`, `assets/index.html:1327`, `assets/index.html:1407-1412`. *Phones:*
  `testNothingToGoOnAndExpiredSayNothingAlarming`.
- **OV-C73** A window with nothing to project from reads "not enough burn yet to project", plain,
  with no basis. *Why:* the forecast refuses rather than invents. *Regression:* no. *Source:*
  `assets/index.html:1242`, `assets/index.html:1260`. *Phones:*
  `testNothingToGoOnAndExpiredSayNothingAlarming`.
- **OV-C74** A window already at 100 % says it has run out, in red, whatever basis its burn came
  from. *Why:* "Already there. Say so rather than projecting a crossing in the past."
  (`src/forecast.rs:279-291`); check "a router that predates the band still renders as it always
  did" keeps its at-the-limit window red. *Regression:* no. *Source:* `assets/index.html:1242-1252`,
  `scripts/check-dashboard-ui.js:189-195`. *Today:* broken — when the router has no fitted burn for
  that window (basis `none`), the page tests the basis first and reads "not enough burn yet to
  project" under a full meter. *Under decision J:* the console ships inside the router it reads, so the older-router case here never reaches it; the rest of the rule stands, and the phones still meet older routers.
- **OV-C75** With no forecast for a window at all — the endpoint is absent, or the account never
  carried a rate-limit header — the meter gets no forecast line, rather than "no reading" under a
  meter that is showing one. *Why:* the comment on `fcLine`; check "every window state produces a
  line, and only the real trouble is red". *Regression:* no. *Source:*
  `assets/index.html:1273-1277`, `scripts/check-dashboard-ui.js:221-227`. *Phones:*
  `testNothingToGoOnAndExpiredSayNothingAlarming`.
- **OV-C76** Every projection says what it was fitted on, as visible text under it: "R% per hour of
  work, fitted on H · M% of the clock spent working". *Why:* "Estimates must say they are estimates,
  and say what they were fitted on"; check "the basis is disclosed, and never claims a fit it did
  not make". *Regression:* no. *Source:* `assets/index.html:1258-1271`,
  `assets/index.html:1279-1282`, `scripts/check-dashboard-ui.js:256-265`. *Phones:*
  `testRunsOutBeforeTheResetIsRed`.
- **OV-C77** A borrowed rate — the window is too young to have a slope — says "nothing burnt yet
  this window, so this is your recent average" and never "fitted on"; an expired window, a basis of
  none or a zero burn discloses nothing. *Why:* a guess and a measurement must not share a typeface
  (`src/forecast.rs:73-84`). *Regression:* no. *Source:* `assets/index.html:1260-1262`,
  `scripts/check-dashboard-ui.js:256-265`. *Phones:* `testABorrowedRateNeverClaimsAFit`.
- **OV-C78** The duty band is disclosed exactly when it has width — "M% of the clock spent working,
  L–H% across your days" — and a window without one discloses one number, never a range of one.
  *Why:* "the spread of your days is why the verdict above is worded the way it is, and hiding it
  makes the wording look like hedging." *Regression:* no. *Source:* `assets/index.html:1264-1270`,
  `scripts/check-dashboard-ui.js:230-240`. *Phones:* `testAStraddlingBandIsAmberAndSaysSo`,
  `testAnOlderRouterStillRendersAsItAlwaysDid`.
- **OV-C79** Hours of work read as hours: minutes under one ("15 min"), one decimal under ten ("2.4
  h"), whole above ("81 h"), and "—" when unknown. *Why:* check "hours read as hours". *Regression:*
  no. *Source:* `assets/index.html:1230-1231`, `scripts/check-dashboard-ui.js:267-272`. *Phones:*
  `testHoursReadAsHours`.
- **OV-C80** Each live window of the serving account gets a burn-down captioned "<window> ·
  burn-down": the window across its own life, its opening time at the left and "resets <time>" at
  the right, observed usage left of now and a dashed projection right of it, with rules at 0 and 100
  % and a faint line at now. *Why:* "The dashed ray is the whole point — where it meets the ceiling
  is the answer." *Regression:* no. *Source:* `assets/index.html:1285-1321`,
  `assets/index.html:1338-1339`.
- **OV-C81** The projection is always drawn: to its crossing on the 100 % line when there is one,
  otherwise to the reset at the level it ends; a crossing, and only a crossing, gets the one marker.
  *Why:* check "the burn-down chart draws for every state that has a window". *Regression:* no.
  *Source:* `assets/index.html:1296-1299`, `assets/index.html:1316-1317`,
  `scripts/check-dashboard-ui.js:274-287`.
- **OV-C82** The projection's colour follows the verdict: muted when it resets first, amber when
  tight, red when it crosses (or crosses on a router with no verdict). *Why:* the chart says what
  the line under the meter says. *Regression:* no. *Source:* `assets/index.html:1308-1309`.
- **OV-C83** When the duty band has width the chart shades the cone between a quiet week and a busy
  one, red-tinted when the verdict crosses and grey otherwise; with no band — the 5-hour window, an
  older router — it draws the single ray and no cone. *Why:* "a ray alone reads as a prediction",
  and a 5-hour window measures the session you are in and must not hedge because the weekly one
  learned to (`src/forecast.rs:496-498`). *Regression:* no. *Source:* `assets/index.html:1300-1306`,
  `scripts/check-dashboard-ui.js:247-253`. *Under decision J:* the console ships inside the router it reads, so the older-router case here never reaches it; the rest of the rule stands, and the phones still meet older routers.
- **OV-C84** A window that cannot be drawn is not: an expired or absent window, one with no start,
  or one whose reset is not after its start draws nothing rather than dividing by zero; values
  outside the window are held to its edges; and the panel survives an account with nothing current.
  *Why:* checks "the burn-down chart draws for every state that has a window" and "the runway panel
  survives an account with nothing current". *Regression:* no. *Source:*
  `assets/index.html:1289-1293`, `assets/index.html:1323-1328`,
  `scripts/check-dashboard-ui.js:285-293`.
- **OV-C85** Two burn-downs sit side by side only when both fit at 250 px, otherwise they stack, and
  each is drawn at the width it is shown so its text never scales. *Why:* the comment on the grid's
  width. *Regression:* no. *Source:* `assets/index.html:118-121`, `assets/index.html:1329-1337`.

### All accounts

- **OV-C86** All accounts appears whenever the forecast lists an account and disappears entirely —
  heading, tiles and cards — when it lists none or cannot be read. *Why:* check "the all-accounts
  panel renders, and hides itself when there is nothing". *Regression:* no. *Source:*
  `assets/index.html:446-448`, `assets/index.html:1359-1365`,
  `scripts/check-dashboard-ui.js:295-300`.
- **OV-C87** All accounts covers every account whatever the page is scoped to; only its usage
  figures follow the time range. *Why:* "a quota belongs to an account, not to whatever you happened
  to spend it on" (`src/api.rs:1004-1008`). *Regression:* no. *Source:* `assets/index.html:2519`,
  `assets/index.html:2531-2537`.
- **OV-C88** Spend, tokens and requests add up across accounts; quota percentages never do, each
  being of its own account's limit, so headroom is totalled in hours of work, and the caption says
  "spend and tokens add up, quota does not". *Why:* the comment on `renderAllAccounts`, and
  `src/api.rs:1158-1163`. *Regression:* no. *Source:* `assets/index.html:1356-1368`,
  `assets/index.html:1390-1395`.
- **OV-C89** Headroom counts quota pools, not credentials: two credentials onto one Anthropic
  account share a quota, reporting the same 7-day reset, and count once, and each pool counts the
  window that runs dry first. *Why:* `src/api.rs:1158-1163` and `src/api.rs:1193`. *Regression:* no.
  *Source:* `assets/index.html:1386-1392`, `src/api.rs:1175-1202`.
- **OV-C90** Five tiles: "Spend, every account" (at API rates, across N accounts), "Tokens read"
  (with what was written back), "Requests" (in the selected range), "Headroom left" (hours of work,
  with the first account to run out or "N quota pools, none projected to run out") and "30-day
  pace". *Why:* each tile's meaning is visible text. *Regression:* no. *Source:*
  `assets/index.html:1370-1396`.
- **OV-C91** The 30-day pace is the last seven days of recorded spend scaled to thirty, whatever
  range the page shows, and says so. *Why:* "A pace is only meaningful against a fixed horizon."
  *Regression:* no. *Source:* `assets/index.html:1396`, `src/api.rs:1054-1066`,
  `src/api.rs:1219-1223`.
- **OV-C92** Headroom turns red only for a window whose verdict crosses; a tight one is amber and
  worded as too close to call. *Why:* only crosses is red (5b2c120). *Regression:* no. *Source:*
  `assets/index.html:1382-1389`, `src/api.rs:1188-1192`, `scripts/fixtures/forecast.json`. *Today:*
  broken — the router picks the first to exhaust from the central estimate and the tile turns red
  for it, so the bundled fixture's tight 7-day window reads "first to go: secondary in …" in red.
- **OV-C93** Headroom names the first account to run out as its card does, by its label, with the
  window and the time. *Why:* one account, one name. *Regression:* no. *Source:*
  `assets/index.html:1387`, `assets/index.html:1393-1394`, `assets/index.html:1428`. *Today:* broken
  — it prints the router id, and the window and time only in its tooltip.
- **OV-C94** One card per account: its label, an "active" tag while it serves new requests, a line
  with its plan, "last request <ago>" or "never seen" and any shared quota; a 5-hour and a 7-day
  meter with percentage and reset, or "—" for a window rolled over or never read; each window's
  forecast line; and a footer of cost, tokens read and requests. *Why:* the quota panel's facts for
  every account at once. *Regression:* no. *Source:* `assets/index.html:1406-1436`.
- **OV-C95** Accounts that share a quota say "shares a quota with <names>" on every card in the
  pool, and the line follows the pool as soon as it changes. *Why:* "Say so rather than letting the
  page read as two independent allowances"; check "two credentials on one quota are named as sharing
  it". *Regression:* yes — the comment at `assets/index.html:1416-1417`: leaving the pool out of the
  redraw "froze that line in place". *Source:* `assets/index.html:1398-1405`,
  `assets/index.html:1416-1426`, `scripts/check-dashboard-ui.js:510-518`.
- **OV-C96** An account removed from the router keeps its card while it has history to report.
  *Why:* "a removed account's history is still reported rather than silently dropped"
  (`src/api.rs:1021-1022`). *Regression:* no. *Source:* `assets/index.html:1362`,
  `src/api.rs:1021-1034`.
- **OV-C97** The captured payload in `scripts/fixtures/forecast.json` — three accounts, two with
  rolled-over windows, one serving with a resets-first 5-hour and a tight 7-day window — renders end
  to end with no "undefined" or "NaN" in any line. *Why:* check "the real /api/forecast payload
  renders end to end". *Regression:* no. *Source:* `assets/index.html:1239-1256`,
  `assets/index.html:1359-1437`, `scripts/check-dashboard-ui.js:496-508`.

### Headline

- **OV-C98** Totals in scope holds eight tiles — Prompt tokens read (the largest), Spend, Requests,
  Output tokens, Served from cache, Cache writes, Time to first token and Errors — each with its
  meaning as visible text under its value. *Why:* the plan's `StatTile`, with each tile's meaning
  visible. *Regression:* no. *Source:* `assets/index.html:450-451`, `assets/index.html:1462-1474`.
- **OV-C99** The largest tile says what it counts and over what span — "<first> → <last> ·
  everything the model read, cached or not" — over a sparkline per bucket; with nothing recorded it
  says "nothing recorded yet — point Claude Code at this router". *Why:* an empty router tells the
  reader how to start. *Regression:* no. *Source:* `assets/index.html:1476-1477`.
- **OV-C100** Spend is subtitled with the two figures that dominate it, cached reads and output, and
  with nothing priced says "nothing priced in scope". *Why:* "The total alone is not interpretable:
  on an agentic workload most of it is cache, billed at a tenth of input." *Regression:* no.
  *Source:* `assets/index.html:1480-1485`.
- **OV-C101** Spend opens into its full split: the total, uncached input, 5-minute cache writes
  (1.25× input), 1-hour cache writes (2× input), cache reads (0.1× input), output (dearest per
  token), the share of the prompt served from cache, requests, and that rates are editable in the
  pricing table. *Why:* 04efe6b: "the tooltip is where the number stops being surprising."
  *Regression:* yes — 04efe6b added this split, and the older call from f715d30, bound after it,
  overwrites it (the plan's defects). *Source:* `assets/index.html:1486-1501`. *Today:* broken — a
  shorter, older tooltip replaces the split.
- **OV-C102** Requests gives the count with "N sessions · M projects" and "K by subagents" when
  there are any; scoped to a session it reads "1 session · <project>". *Why:* a request count means
  little without where the requests came from. *Regression:* no. *Source:*
  `assets/index.html:1502-1505`.
- **OV-C103** Output tokens gives the count, how much of it was thinking, and a sparkline per
  bucket. *Why:* thinking is part of the output, and the subtitle says how much. *Regression:* no.
  *Source:* `assets/index.html:1506-1507`.
- **OV-C104** Served from cache is the hit rate — cache reads ÷ every prompt token read — with a
  meter and "X of Y came from cache"; with no prompt tokens it says "no prompt tokens yet". *Why:*
  one definition of the hit rate, the one the read panel's note spells out. *Regression:* no.
  *Source:* `assets/index.html:1508-1510`, `assets/index.html:1601`.
- **OV-C105** Cache writes gives the total with its 5-minute and 1-hour tiers. *Why:* the tiers are
  priced differently, at 1.25× and 2× input. *Regression:* no. *Source:* `assets/index.html:1511`.
- **OV-C106** Time to first token gives the median, with the p95 and the whole reply's median under
  it, and every latency figure — first token p50 and p95, whole reply p50 and p95, and the slowest
  reply — is readable without hovering; with no requests it reads "median · no requests yet". *Why:*
  the plan lists p95 latencies among the facts a phone cannot reach. *Regression:* no. *Source:*
  `assets/index.html:1512-1513`, `src/api.rs:457-469`. *Today:* broken — the whole reply's p95 and
  the slowest reply are in the tooltip only.
- **OV-C107** Errors gives the count, red when above zero, "of N requests"; with any errors it
  offers a visible, keyboard-reachable "List the N errors" button that sets the request filter to
  errors, returns that list to its first page and brings it into view. *Why:* the plan's visible
  button. *Regression:* no. *Source:* `assets/index.html:1514-1520`. *Today:* broken — the whole
  tile is a clickable `div` with no role or tab stop, worded "click to list them".
- **OV-C108** The heading's caption reads "N requests · <last seen>" once anything is recorded.
  *Why:* how much and how recent, at a glance. *Regression:* no. *Source:* `assets/index.html:1521`.

### Activity

- **OV-C109** Activity stacks each bucket's prompt tokens by model family, opus at the bottom then
  sonnet, haiku and other, in the family colours; its legend keeps that order, dims a family absent
  from the range rather than dropping it, never filters the chart when pressed, and says "prompt
  tokens per <bucket>". *Why:* the plan asks for a fixed legend order and a legend that does not
  filter, which Carbon Charts does by default. *Regression:* no. *Source:*
  `assets/index.html:453-457`, `assets/index.html:1529-1531`, `assets/index.html:1545-1554`.
- **OV-C110** The chart spans the selected range, quiet stretches included: every bucket from start
  to end is drawn, an empty one as empty; with the range all it spans the data; with no requests in
  a range it still shows the range. *Why:* "so a quiet hour still shows as an hour of quiet"
  (`src/api.rs:471-472`). *Regression:* no. *Source:* `assets/index.html:1440-1456`,
  `src/api.rs:471-493`. *Phones:* `testSummaryDecodesAndBucketsFillTheRange`.
- **OV-C111** Buckets follow the viewer's clock — the page sends its UTC offset, so a day bar starts
  at the viewer's midnight — and two models' rows in one bucket add up. *Why:* `src/api.rs:59-60`.
  *Regression:* no. *Source:* `assets/index.html:1442-1452`, `assets/index.html:2504`,
  `assets/index.html:2511`. *Phones:* `testSummaryDecodesAndBucketsFillTheRange`.
- **OV-C112** The bucket width is the router's — the smallest of 1, 5, 15 or 30 minutes, 1, 3 or 6
  hours, or a day that keeps the span under about 72 bars — and the page draws at most 500 buckets.
  *Why:* "so a ten-minute session shows minutes and a month shows days." *Regression:* no. *Source:*
  `assets/index.html:1454`, `src/api.rs:92-110`.
- **OV-C113** Two empty charts are told apart: "No requests in this range." and "N requests, none of
  which reported any tokens." *Why:* which empty it is. *Regression:* no. *Source:*
  `assets/index.html:1532-1535`.
- **OV-C114** The y axis tops at a round number — 1, 2 or 5 times a power of ten — above the tallest
  bar and is labelled at half and top; time labels never collide, the last bucket is always
  labelled, and they read as dates for day buckets, date and time across more than a day, and time
  otherwise. *Why:* axis text that overlaps or reads oddly is noise. *Regression:* no. *Source:*
  `assets/index.html:909`, `assets/index.html:1536-1560`.
- **OV-C115** A bucket with errors carries a mark above its bar in the error colour. *Why:* errors
  are found by time as well as by count. *Regression:* no. *Source:* `assets/index.html:1555`.
- **OV-C116** Pointing at a bucket draws a crosshair and gives its span, each family's tokens and
  requests, output tokens, cost, the share from cache, the share by subagents and its errors, and
  the same figures are reachable by tap and by keyboard. *Why:* the plan lists per-bucket values
  among the facts a phone cannot reach. *Regression:* no. *Source:* `assets/index.html:1567-1585`.
  *Today:* broken — the figures follow the pointer only, so a tap at most flashes them and the
  keyboard never reaches them.
- **OV-C117** The caption gives the range's prompt tokens and requests. *Why:* the chart's total,
  stated. *Regression:* no. *Source:* `assets/index.html:1566`.

### Read and context

- **OV-C118** What the model read is one share bar of three parts in a fixed order on the token ramp
  — from cache (darkest), written to cache, paid in full (lightest) — with a legend of tokens and
  share of everything read, the write row split into its 5-minute and 1-hour tiers. *Why:* cache
  against paid-in-full is the split that decides cost and quota. *Regression:* no. *Source:*
  `assets/index.html:459-465`, `assets/index.html:1590-1600`.
- **OV-C119** Paid in full is the router's uncached input; for a router that does not send it, it is
  everything read less cache reads and writes, never below zero. *Why:* older routers still draw a
  whole bar. *Regression:* no. *Source:* `assets/index.html:1592`. *Under decision J:* the console ships inside the router it reads, so the older-router case here never reaches it; the rest of the rule stands, and the phones still meet older routers.
- **OV-C120** The note under it gives the hit rate as its formula — "Hit rate = cache reads ÷
  everything read = N%." — and that cached tokens are far cheaper against the quota than tokens paid
  in full; with nothing read it says "Nothing read yet." *Why:* the explanation sits next to the
  figure. *Regression:* no. *Source:* `assets/index.html:1601`.
- **OV-C121** The context window has four components in a fixed order with fixed labels, colours and
  icons — tools (definitions), system prompt, conversation, tool results — on the panel and in a
  request's detail alike. *Why:* "the same four meanings" (`assets/index.html:15`). *Regression:*
  no. *Source:* `assets/index.html:659-664`, `assets/index.html:1606-1616`,
  `assets/index.html:1925-1929`.
- **OV-C122** The context split is labelled an estimate where it is shown — "estimate · by character
  share" — with the note that totals are exact and the split apportions the API's exact prompt-token
  count by character share, there being no local tokenizer for current Claude models, so ratios are
  reliable and absolutes approximate. *Why:* estimates say they are estimates. *Regression:* no.
  *Source:* `assets/index.html:467-470`.
- **OV-C123** Each component row gives its estimated tokens, its share of the window and "≈ per
  request", divided by the requests that carried that component (by all requests only for a router
  that does not say), over a bar of its share. *Why:* an average over requests that did not carry it
  would understate it. *Regression:* no. *Source:* `assets/index.html:1609-1621`, `src/api.rs:376`.
- **OV-C124** A context or tool-output cost reads as the floor it is — "≥$X", "if paid once as
  input" — beside its figure, with the character count, readable without hovering. *Why:* it is
  priced from estimated tokens as if paid once as fresh input, while whatever stays in the window is
  re-read every later turn (04efe6b; `src/api.rs:367-370`, `src/api.rs:775-778`). *Regression:* no.
  *Source:* `assets/index.html:1610`, `assets/index.html:1778-1780`. *Today:* broken — a context
  component's cost and characters, and a tool's characters and largest result, are in tooltips only.
- **OV-C125** Pointing at a component row lights its part of the bar, and pointing at a part lights
  it and gives its figures. *Why:* the row-to-segment highlight the plan keeps in `ShareBar`.
  *Regression:* no. *Source:* `assets/index.html:1622-1623`, `assets/index.html:834-844`.

### Models and threads

- **OV-C126** By model has one row per model, ranked by prompt tokens read: a family swatch, the
  short name with the full id one press away, a bar in the family colour, and "N req · out · %
  cached · cost", with subagent requests and errors when there are any; empty, it says "No requests
  in scope." *Why:* the models that read the most lead. *Regression:* no. *Source:*
  `assets/index.html:1629-1643`, `src/api.rs:387-406`.
- **OV-C127** Main thread vs subagents is exactly two parts, main and subagents, as a share bar in
  slate and the subagent colour with their icons; requests from before agent ids were recorded count
  as main, never as a third, nameless thread. *Why:* `src/api.rs:407-408`. *Regression:* no.
  *Source:* `assets/index.html:1646-1654`, `src/api.rs:407-427`.
- **OV-C128** Each thread's legend line gives its tokens and share, then "N req · out · cost · %
  cached", and for subagents how many distinct agents; with none it says "none in scope". *Why:* the
  split is read with its size. *Regression:* no. *Source:* `assets/index.html:1655-1659`.

### Lists

- **OV-C129** Every list is bar rows — a label line over a magnitude bar scaled to the largest row
  in it — never a table, and never wider than a phone. *Why:* the user's standing preference for
  bars and compact rows over wide tables, and the plan's `BarList`, never a `DataTable`.
  *Regression:* no. *Source:* `assets/index.html:207-242`, `assets/index.html:852-886`.
- **OV-C130** A list that scopes the page marks its selected row — pressed, with an accent edge —
  dims the others while leaving them pressable, and says in its caption how it works: "click one to
  scope the page", then "click the highlighted … again to clear". *Why:* the explanation sits next
  to the control. *Regression:* no. *Source:* `assets/index.html:214-215`,
  `assets/index.html:1672-1675`, `assets/index.html:1703-1704`, `assets/index.html:1745`.
- **OV-C131** A navigator is never narrowed by the level it picks: the Accounts list and the account
  picker follow the range only, the Projects list the account and range, the Sessions list the
  account, project and range. *Why:* "Never narrowed by project or session (it is how you pick one)"
  (`src/api.rs:547-548`, `src/api.rs:581`). *Regression:* no. *Source:*
  `assets/index.html:2512-2513`, `src/api.rs:428-446`, `src/api.rs:547-609`.
- **OV-C132** Lists keep the router's order: projects, models and accounts by prompt tokens, tool
  output by estimated tokens, tool calls by count, sessions by last activity, requests newest first.
  *Why:* the largest or the latest leads, and a row keeps its place across polls while that holds.
  *Regression:* no. *Source:* `assets/index.html:754-761`, `src/api.rs:576`, `src/api.rs:608`,
  `src/api.rs:757`.
- **OV-C133** The Accounts list appears only once more than one account has served traffic in the
  range. *Why:* "with a single account it would restate the totals directly above it." *Regression:*
  no. *Source:* `assets/index.html:486-487`, `assets/index.html:1662-1671`.
- **OV-C134** An account row shows the account icon, its label and router id, an "active" tag while
  it serves, a bar of prompt tokens, and "N sessions · N req · out · <last seen>". *Why:* the facts
  that tell two accounts apart. *Regression:* no. *Source:* `assets/index.html:1684-1694`.
- **OV-C135** A project row: the project icon in the project colour, the folder's name, its path,
  one swatch per model family it used (four at most), a bar of prompt tokens with the subagent share
  laid over its end and labelled "N% subagents", and "N sessions · req · out · % cached · cost ·
  errors · <last seen>". *Why:* a project's size, mix and recency in one row. *Regression:* no.
  *Source:* `assets/index.html:1700`, `assets/index.html:1709-1719`.
- **OV-C136** A project with no recorded path is listed as "(unresolved)" and scopes like any other,
  the router selecting exactly those sessions for it. *Why:* "no real working directory is spelt
  that way, so it selects them rather than nothing" (`src/api.rs:67-68`). *Regression:* no.
  *Source:* `assets/index.html:715`, `assets/index.html:1060-1062`, `assets/index.html:1709`,
  `src/api.rs:64-72`.
- **OV-C137** A session row: the session icon in the session colour, the first eight characters in
  monospace, chips for its git branch, its project (only when no project is scoped), the client it
  was launched from and its model families, a bar with the subagent share laid over it, and "N req ·
  cost · out · N subagents · errors · started <time> · <duration>". *Why:* enough to recognise a
  session without opening it. *Regression:* no. *Source:* `assets/index.html:1747-1762`. *Phones:*
  `testListsDecode`.
- **OV-C138** The Sessions caption names the scope and the count, and says "200 most recent" when
  the router's cap is reached. *Why:* truncation is loud. *Regression:* no. *Source:*
  `assets/index.html:1745`, `src/api.rs:608`.
- **OV-C139** A long list shows a first page and a visible "Show more", as Requests does, rather
  than drawing every row; Sessions holds up to 200. *Why:* Northstar 7b6e4ab: a capped 200-row list
  made a page 30,803 px tall. *Regression:* no. *Source:* `assets/index.html:1739-1766`. *Today:*
  broken — all 200 session rows are drawn at once.
- **OV-C140** Requests that arrive before their transcript have no session and so no project; a "not
  yet attributed" row counts them under Projects, and under Sessions when no project is scoped, so
  the lists never sum to less than the tiles; it is absent when there are none or a session is
  scoped. *Why:* "Without this row the lists would silently sum to less than the tiles."
  *Regression:* no. *Source:* `assets/index.html:1721-1738`, `assets/index.html:1764`,
  `src/api.rs:64-66`.
- **OV-C141** "Not yet attributed" counts only requests with no session, never rows a list left out
  at its cap. *Why:* it rests on every figure coming from one snapshot (`src/api.rs:302-305`), and
  the plan moves the count to the router (AGEN-0015). *Regression:* no. *Source:*
  `assets/index.html:1727-1728`, `src/api.rs:606-608`. *Today:* broken — it subtracts the listed
  sessions from the total, so once the Sessions list is capped at 200 every request of an older
  session counts there as unattributed.
- **OV-C142** The unattributed row explains itself beside it: Claude Code labels a request with its
  session id, the project comes from the transcript file, which can appear a moment later, and these
  requests are counted in the totals above. *Why:* the explanation sits next to the figure.
  *Regression:* no. *Source:* `assets/index.html:1733-1736`. *Today:* broken — only "no project
  recorded yet" is visible; the why, and that these requests are in the totals, are in its tooltip.
- **OV-C143** Tool output the model read has one row per tool, ranked by estimated tokens, with "N
  results · ≈X each · ≥$cost", and "N failed" in the error colour with the error icon. *Why:* which
  tools fill the context, and at what floor of cost. *Regression:* no. *Source:*
  `assets/index.html:1771-1783`.
- **OV-C144** Tool calls made has one row per tool, ranked by calls, its count exact rather than
  compacted, the calls made by subagents laid over the bar's end and labelled "N by subagents", and
  "N calls · in M requests". *Why:* calls are counted, not measured. *Regression:* no. *Source:*
  `assets/index.html:1784-1795`.
- **OV-C145** Every empty panel says which empty it is: nothing recorded at all, with how to start
  ("No projects yet — point Claude Code at this router and run it."); nothing in the range ("No
  project sent a request in the last 24h.", "No session ran in the last 7d."); nothing in this
  scope; or no quota reading yet for the serving account. *Why:* an empty list says which empty it
  is (Northstar §8). *Regression:* no. *Source:* `assets/index.html:1157`,
  `assets/index.html:1705-1707`, `assets/index.html:1743-1744`, `assets/index.html:1772`,
  `assets/index.html:1784`, `assets/index.html:1601`. *Today:* broken — the tool panels and What the
  model read say "yet" ("No tool results captured yet.", "Nothing read yet.") whatever the scope or
  range.

### Requests

- **OV-C146** The list holds the 300 newest requests in scope; the filters narrow what is loaded,
  and the caption says so: "X of Y shown", "filtered from N", and "300 newest loaded" at the cap.
  *Why:* "Filters narrow what is already loaded; the API hands back the 300 newest at most."
  *Regression:* no. *Source:* `assets/index.html:1815`, `assets/index.html:1854-1858`,
  `src/api.rs:757`.
- **OV-C147** The cap's note offers a remedy only where one exists: "scope to a session for the
  rest" when no session is scoped. *Why:* advice that cannot work is noise. *Regression:* no.
  *Source:* `assets/index.html:1857`. *Today:* broken — a session with more than 300 requests is
  told to scope to a session.
- **OV-C148** The kind filters are All, Errors, Subagents and Slow, each with its count; a kind
  shows only when some loaded row is of it, All always. *Why:* a filter that could only answer
  nothing is not offered. *Regression:* no. *Source:* `assets/index.html:1809-1833`.
- **OV-C149** An error is one thing everywhere — a status of 400 or more, or an error recorded — and
  a subagent request is one marked as a sidechain: the tiles, the chart's marks, the lists' counts,
  the row badges and the filters count the same rows. *Why:* the router counts them so
  (`src/api.rs:333-334`). *Regression:* no. *Source:* `assets/index.html:1811-1812`,
  `assets/index.html:1893-1896`, `src/api.rs:333-334`. *Phones:* `testListsDecode`.
- **OV-C150** Slow means a first token slower than the scope's p95, and the filter states that
  threshold in visible text ("first token slower than the p95 of 3.2 s"); without a p95 there is no
  Slow. *Why:* the plan wants the p95 threshold as visible helper text. *Regression:* no. *Source:*
  `assets/index.html:1813`, `assets/index.html:1818`. *Today:* broken — the threshold is only in the
  chip's tooltip.
- **OV-C151** The slow badge and the Slow filter agree: every row the filter keeps carries the
  badge. *Why:* a filter must show why each row is in it. *Regression:* no. *Source:*
  `assets/index.html:1813`, `assets/index.html:1894`. *Today:* broken — an error row slower than the
  p95 is counted under Slow but its badge is suppressed.
- **OV-C152** Model filters appear when more than one model is loaded — a chip per model with its
  family swatch, short name and count, the full id one press away — and stop-reason filters when
  more than one stop reason is, "(none)" standing for a request with none; each toggles. *Why:* a
  filter with one value filters nothing. *Regression:* no. *Source:* `assets/index.html:1820-1845`.
- **OV-C153** A filter whose value has left the loaded rows is cleared, never left filtering to
  nothing, and a kind filter that is on always shows its chip pressed. *Why:* a filter that is on
  and cannot be seen explains nothing. *Regression:* no. *Source:* `assets/index.html:1822-1823`,
  `assets/index.html:1832-1833`, `assets/index.html:1848`. *Today:* broken for the kind — its chip
  disappears once no loaded row matches (after scoping to a session without errors, say) while the
  filter stays on, and the list reads "No request matches these filters." with nothing pressed.
- **OV-C154** Any filter change returns the list to its first twenty rows; "Show N more" adds twenty
  at a time and "Show fewer" returns to twenty, as buttons, never pagination. *Why:* the plan's
  "Show more" is a tertiary button, and a 300-row list drawn whole makes a very long page (Northstar
  7b6e4ab). *Regression:* no. *Source:* `assets/index.html:1800`, `assets/index.html:1829`,
  `assets/index.html:1860-1869`.
- **OV-C155** With filters that leave nothing, the list says "No request matches these filters."
  beside a "Clear the filters" button; with no requests at all, "No requests for this scope." *Why:*
  which empty it is, and the way out. *Regression:* no. *Source:* `assets/index.html:1804-1807`,
  `assets/index.html:1870-1875`.
- **OV-C156** A request row reads its time (with seconds today, with the date otherwise, the full
  date-time one press away), the family swatch and short model, a subagent tag and a slow badge when
  they apply, its status (an error tag with the code, or "ERR"), a bar of its prompt tokens against
  the largest in the filtered set, the prompt tokens, the share cached ("—" without a prompt),
  output and cost, and the time to first token. *Why:* a request recognised at a glance.
  *Regression:* no. *Source:* `assets/index.html:1876-1902`.
- **OV-C157** A row opens in place into its detail, and open rows stay open across polls and
  reloads, being in the address. *Why:* the five-second refresh must not close what the reader
  opened. *Regression:* no. *Source:* `assets/index.html:1884`, `assets/index.html:1904-1908`,
  `assets/index.html:914-920`.
- **OV-C158** The detail gives the request id, the session and its project (unless a session is
  scoped), the whole reply's time with the first token's, the stop reason, thinking tokens, cache
  read · written · uncached, the agent id of a subagent, the tools called in the order first called
  with "×N" for repeats, the error at full width with its line breaks, and the request's context
  window as a share bar with each component's percentage. *Why:* everything recorded about one
  request. *Regression:* no. *Source:* `assets/index.html:1909-1931`, `assets/index.html:262-269`.
  *Phones:* `testListsDecode`.
- **OV-C159** On a phone a request is a two-line card — time, model, badges and status over the bar,
  prompt, cache and output — that never widens the page, the first-token time moving into the
  detail. *Why:* the plan's phone-first `RequestRow`. *Regression:* no. *Source:*
  `assets/index.html:247-249`, `assets/index.html:354-357`. *Today:* broken — the eleven-track row
  is one of the causes of the sideways scroll at 360 px.

### Refresh

- **OV-C160** One snapshot: every figure on the Overview comes from one read, scoped by the same
  account, project, session and range, so the tiles, charts and lists agree, and a cycle is drawn
  whole or not at all. *Why:* "so every figure on the page agrees with every other"
  (`src/api.rs:302-305`); the unattributed row rests on it. *Regression:* no. *Source:*
  `assets/index.html:2503-2549`, `src/api.rs:302-305`.
- **OV-C161** The next read starts five seconds after the previous one finished, so reads never
  overlap or pile up, and the router's one-second memo stays shorter than that, so nothing shown is
  staler than it already was. *Why:* `src/api.rs:212-218`. *Regression:* no. *Source:*
  `assets/index.html:2652-2657`, `src/api.rs:212-218`, `src/proxy.rs:127-129`.
- **OV-C162** A reply for a scope that is no longer current is never drawn. *Why:* the figures must
  belong to the crumbs above them. *Regression:* no. *Source:* `assets/index.html:932-938`,
  `assets/index.html:2503-2552`. *Today:* broken — a scope change starts a read while the old one is
  in flight, and whichever answers last paints, so the old scope's figures can land under the new
  crumbs.
- **OV-C163** While a new scope is read, the previous figures stay on screen; nothing blanks. *Why:*
  the plan's poll keeps the previous data on a scope change. *Regression:* no. *Source:*
  `assets/index.html:932-938`, `assets/index.html:2524-2549`.
- **OV-C164** A failed cycle draws nothing and keeps everything drawn; consecutive failed cycles —
  not HTTP attempts — mark the page stale with the time of the last good read, five in a row mark
  the router unreachable, a good cycle clears both, and nothing is retried quietly. *Why:* data kept
  from a failed refresh is labelled, never blanked (Northstar §1), and the plan's poll never retries
  quietly. *Regression:* no. *Source:* `assets/index.html:1129-1136`, `assets/index.html:2652-2656`.
- **OV-C165** Every read has a timeout, and one that runs out fails its cycle: a hung request never
  stops the poll, and the page never says live over a read that has not come back. *Why:* the plan
  gives every request a timeout. *Regression:* no. *Source:* `assets/index.html:809-814`,
  `assets/index.html:2652-2657`. *Today:* broken — reads have no timeout, so one that never answers
  stops the loop while the chip still says "live".
- **OV-C166** Settings never fail the Overview: the accounts list and the router's mode are read
  every 30 s beside the snapshot — the mode before the accounts are drawn, since it decides what the
  accounts panel may interrupt for — and a failure in either leaves the Overview's figures and
  freshness alone. *Why:* "whether this is a hosted router decides what the accounts panel may
  interrupt for." *Regression:* no. *Source:* `assets/index.html:2506-2509`,
  `assets/index.html:2516`, `assets/index.html:2520-2523`, `assets/index.html:2550`. *Today:* broken
  — an `/api/accounts` failure, or a throw while drawing the accounts table, fails the whole cycle,
  and the chip reads stale over fresh figures.
- **OV-C167** Credentials are probed only while Settings is open, and the 30-second accounts read is
  otherwise cheap. *Why:* "Resolving every credential is not free, so it is done when the panel is
  actually open." *Regression:* no. *Source:* `assets/index.html:947-948`, `assets/index.html:2506`,
  `assets/index.html:2516`, `src/api.rs:1326-1337`. *Today:* broken — the router ignores `probe=1`
  and checks every credential on every call (AGEN-0015).
- **OV-C168** An action that changes what the Overview shows — an account switched, added or removed
  — asks the one poll to read now; nothing starts a second read beside it. *Why:* one clock drives
  every re-read (Northstar §6). *Regression:* no. *Source:* `assets/index.html:1990`,
  `assets/index.html:2000`, `assets/index.html:2223`, `assets/index.html:2258`. *Today:* broken —
  these actions call the read directly, outside the loop's error handling, so a failure there is an
  unhandled rejection and two reads can run at once.
- **OV-C169** The Overview re-reads at once when its tab becomes visible again. *Why:* Northstar §6,
  which the plan binds here. *Regression:* no. *Source:* `assets/index.html:2652-2661`. *Today:*
  broken — nothing listens for it, so a returning tab shows what its throttled background timer last
  read until the next tick.
- **OV-C170** The five-second refresh keeps the reader's place — focus, hover, scroll, open request
  rows, an open picker and a lit segment survive it — because each row is the same row from one poll
  to the next: a project by its path, a session or request by its id, a model, tool or account by
  its name or id. *Why:* "rows keep their DOM nodes across polls, so hover, focus and expansion
  survive". *Regression:* no. *Source:* `assets/index.html:746-764`, `assets/index.html:997-1006`.
- **OV-C171** A poll that returns the same figures redraws nothing: charts, share bars, cards and
  the burn-down change only when their data or their width did, so a crosshair or a lit segment is
  not reset every five seconds. *Why:* Carbon Charts calls `setData` whenever the data array's
  identity changes, so chart data is memoised by content (the plan's known traps). *Regression:* no.
  *Source:* `assets/index.html:824-830`, `assets/index.html:908`, `assets/index.html:1201-1204`,
  `assets/index.html:1334-1337`, `assets/index.html:1416-1421`, `assets/index.html:1564`.
- **OV-C172** Charts and bar labels redraw at their new width when the window or a panel resizes.
  *Why:* every chart is drawn to the width it is shown. *Regression:* no. *Source:*
  `assets/index.html:1587`.
- **OV-C173** Every figure can say when it was read: the snapshot's time is on the page, and a value
  kept from an earlier read carries its own age. *Why:* a display that cannot say when it was
  measured is as defective as a count that cannot say what it counted (Northstar §1). *Regression:*
  no. *Source:* `assets/index.html:1131-1135`, `assets/index.html:1178`,
  `assets/index.html:2529-2530`. *Today:* broken — only the quota's "checked … ago" and, after a
  failure, the chip's "stale · last …" say when; the forecast, tiles and lists carry no time.

### Formatting

- **OV-C174** Token quantities are compact: whole below a thousand, one decimal in thousands below
  ten thousand ("2.4k"), whole thousands below a million ("120k"), two decimals in millions and
  billions ("1.20M", "2.68B"); a value is rounded before the next threshold is tested, so 999,600
  reads "1.00M", never "1000k". *Why:* the comment in `k`. *Regression:* yes — the comment at
  `assets/index.html:671` records "1000k". *Source:* `assets/index.html:670-676`. *Phones:*
  `testHoursReadAsHours`.
- **OV-C175** A token figure the router did not record reads "—", and only a recorded zero reads
  "0". *Why:* missing is not zero (Northstar §1). *Regression:* no. *Source:*
  `assets/index.html:670`, `assets/index.html:1899`. *Today:* broken — the page's compact formatter
  turns a missing value into "0" (a request with no usage row shows "0" prompt and "0 out"), where
  the phone's prints "—".
- **OV-C176** Counts — requests, sessions, calls, results, errors — are exact with thousands
  separators, never compacted, and agree in number ("1 session", "2 sessions"). *Why:* a count is
  not a measurement. *Regression:* no. *Source:* `assets/index.html:669`, `assets/index.html:1479`,
  `assets/index.html:1789`.
- **OV-C177** Dollars read whole with separators from $1,000, in cents from $1, to three places
  below that and four below a cent, "$0" for zero, and "—" for a cost that could not be priced,
  never "$0". *Why:* "Cents matter at coffee prices and are noise at four figures, and a cost we
  could not price — an unknown model — reads as an em dash rather than as free." *Regression:* no.
  *Source:* `assets/index.html:677-683`. *Phones:* `testHoursReadAsHours`.
- **OV-C178** A total that includes requests the router could not price says so, and one with
  nothing priced reads "—" rather than "$0". *Why:* an unpriced model "contributes nothing rather
  than counting as free" (`src/pricing.rs:122-124`), so its absence must show. *Regression:* no.
  *Source:* `assets/index.html:1484-1485`, `src/api.rs:325`. *Today:* broken — the router sums
  unpriced requests as nothing, so a mixed scope reads low with no mark, and an unpriced one reads
  "$0" over "nothing priced in scope".
- **OV-C179** A share reads "—" when there is nothing to divide by, whole from 10 % and at 0, and to
  one decimal below 10 %, so 27 errors in 8,474 requests read 0.3 %, not 0 %. *Why:* a small share
  is not none. *Regression:* no. *Source:* `assets/index.html:684-685`. *Phones:*
  `testRangesAndFormats`.
- **OV-C180** A request's timing reads "812 ms" below a second and "3.2 s" above. *Why:* one
  spelling across tiles, rows and details. *Regression:* no. *Source:* `assets/index.html:686`.
  *Phones:* `testRangesAndFormats`.
- **OV-C181** Spans read "45s", "3m 05s", "3h 12m", "1d 0h"; a time to come reads "in 5 min", "in 2h
  10m", "in 3d 4h", or "now" once it has passed; a time gone reads "just now" under 45 seconds, then
  "N min ago", "N h ago", "N d ago" for a week, then the date. *Why:* the phones copy these by name
  (`Fmt.duration`, `Fmt.relative`, `Fmt.ago`). *Regression:* no. *Source:*
  `assets/index.html:687-690`, `assets/index.html:697-700`, `assets/index.html:708-710`. *Phones:*
  `testRunsOutBeforeTheResetIsRed`.
- **OV-C182** A moment reads as its time of day when it is today and as its date and time otherwise.
  *Why:* today's date is noise. *Regression:* no. *Source:* `assets/index.html:691-696`,
  `assets/index.html:1889`.
- **OV-C183** A reset instant is read in both spellings the router sends — "@<epoch seconds>" from
  stored headers and an ISO date from the usage API — and anything else is unknown, never the epoch.
  *Why:* the two sources spell it differently (`src/quota.rs:337`). *Regression:* no. *Source:*
  `assets/index.html:701-707`.
- **OV-C184** A model's short name is the same on every client. *Why:* the formatters are the ones
  the phones copy by name. *Regression:* no. *Source:* `assets/index.html:713-714`,
  `ios/AgentRouter/Sources/Views/Format.swift:107-111`. *Phones:* `testRangesAndFormats`. *Today:*
  broken — the page drops "claude-" and the date and joins the version ("claude-opus-4-5-20251101"
  reads "opus 4.5", "claude-opus-5-20260115" reads "opus 5"), while the phone only drops "claude-"
  ("opus-5-20260115"). *Decided:* the phones' spelling, on 2026-10-05 (decision K in the plan).
- **OV-C185** A bucket is named the same on every client: "minute", "hour", "day", and one spelling
  for their multiples. *Why:* the phone's own comment says it uses "the dashboard's bucket names".
  *Regression:* no. *Source:* `assets/index.html:719`,
  `ios/AgentRouter/Sources/Views/Format.swift:95-105`. *Phones:* `testRangesAndFormats`. *Today:*
  broken — the page says "15 min" where the phone says "15 minutes". *Decided:* the phones' spelling, on 2026-10-05 (decision K in the plan).
- **OV-C186** A project is named by the last part of its path, "(unresolved)" when it has none, with
  the whole path available beside it; a session by the first eight characters of its id, the whole
  id available. *Why:* short enough for a row, exact one press away. *Regression:* no. *Source:*
  `assets/index.html:715-716`. *Phones:* `testListsDecode`, `testSearchResultsDecode`.
- **OV-C187** The client a session ran in is named "VS Code" when its entrypoint mentions
  claude-vscode, else "CLI" when it mentions claude-cli, else not named, with the full entrypoint
  available. *Why:* the VS Code extension launches the CLI, so the VS Code mark is tested first.
  *Regression:* no. *Source:* `assets/index.html:717`, `assets/index.html:1754`. *Phones:*
  `testListsDecode`.
- **OV-C188** Everything the router sends reaches the page as text, never as markup — transcripts,
  snippets, paths, model names, labels and errors — and text placed in a drawn chart is escaped.
  *Why:* "all data goes through textContent"; the live view's quote-escaping hole (29c5eb3) is what
  markup from data costs. *Regression:* no. *Source:* `assets/index.html:721-734`,
  `assets/index.html:711`, `assets/index.html:1318-1319`.

### Shared visuals

- **OV-C189** A share bar draws its parts in a fixed order, leaves out a part of zero, and draws any
  part above zero at least 3 px wide. *Why:* a sliver must still be seen. *Regression:* no.
  *Source:* `assets/index.html:162-169`, `assets/index.html:824-830`.
- **OV-C190** A thin part of a share bar is still easy to point at: a part under 24 px wide owns a
  24 px zone around its centre, the bar's pointing zone reaches 8 px above and below it, and the
  part under the pointer is outlined and its figures shown. *Why:* "narrow segments win a 24px zone
  around their centre so a sliver is still hoverable". *Regression:* no. *Source:*
  `assets/index.html:163`, `assets/index.html:170`, `assets/index.html:834-844`.
- **OV-C191** A bar row's bar is scaled to the largest row in its list without dividing by zero; a
  zero draws as a faint stub rather than nothing, and any value above zero at least 3 px. *Why:*
  zero and absent read differently, and a small value is still seen. *Regression:* no. *Source:*
  `assets/index.html:224-228`, `assets/index.html:871-874`, `assets/index.html:1632`.
- **OV-C192** The subagent share is laid over the end of its bar in the subagent colour, set off by
  a 2 px gap, and labelled. *Why:* every row's main/subagent split at a glance. *Regression:* no.
  *Source:* `assets/index.html:229`, `assets/index.html:852-857`, `assets/index.html:871-879`.
- **OV-C193** A bar's value label sits just past the bar's end and moves inside the bar when it
  would run past the track, whatever its length, and again after a resize. *Why:* "Measured, because
  the text length varies per row." *Regression:* no. *Source:* `assets/index.html:230-233`,
  `assets/index.html:877-897`, `assets/index.html:1587`.
- **OV-C194** A sparkline is one bar per bucket scaled to the largest, any bucket above zero at
  least a pixel tall and an empty one blank, over a baseline, the last bucket — the one still
  filling — in the darker step. *Why:* the plan's `Sparkline` marks only the last point.
  *Regression:* no. *Source:* `assets/index.html:899-907`, `assets/index.html:1477`,
  `assets/index.html:1507`.

### Responsive and accessible

- **OV-C195** The page never scrolls sideways at 360, 375 or 414 px. *Why:* the same product on a
  phone (the plan's responsive rules). *Regression:* no. *Source:* `assets/index.html:157`,
  `assets/index.html:221-222`, `assets/index.html:247`, `assets/index.html:347-359`. *Today:* broken
  — readers measured 143, 128 and 89 px of sideways scroll, from bar-row meta lines that never wrap,
  the session chips, the request row's grid and the 330 px two-up minimum.
- **OV-C196** Long unbroken text — paths, ids, model names, errors, snippets — wraps anywhere or
  ellipsises and never widens the page. *Why:* Northstar 4b65e8b: one unbroken URL widened a 390 px
  page to 1,119 px. *Regression:* no. *Source:* `assets/index.html:201-202`,
  `assets/index.html:219-220`, `assets/index.html:257`, `assets/index.html:265-267`.
- **OV-C197** Every tooltip on the Overview opens by tap and by keyboard focus and can be pinned,
  and a fact the phone layout drops — a project's path, an account's id — is shown another way.
  *Why:* no fact reachable only by hover (the plan's `DataTip`). *Regression:* no. *Source:*
  `assets/index.html:349`, `assets/index.html:1686`, `assets/index.html:1713`. *Today:* broken —
  tooltips follow the pointer and vanish when it leaves, so a tap only flashes them; those on
  charts, share bars and non-selectable rows never take keyboard focus; and the path and account id
  are hidden at 700 px and below.
- **OV-C198** On a touch device every field is at least 16 px and every target at least 48 px.
  *Why:* iOS zooms into a field under 16 px and never back out, and Northstar sizes by pointer type,
  not by width. *Regression:* no. *Source:* `assets/index.html:71-73`, `assets/index.html:82-83`,
  `assets/index.html:98-99`, `assets/index.html:189`. *Today:* broken — the search box is 14 px, a
  picker's filter 13 px, the range buttons 28 px tall, and the crumbs and chips smaller still.
- **OV-C199** Text meets 4.5:1 and marks 3:1 against what they sit on, in both themes, labels inside
  bars and faint captions included. *Why:* the plan's contrast pass. *Regression:* no. *Source:*
  `assets/index.html:10-11`, `assets/index.html:26-27`, `assets/index.html:231-233`. *Today:* broken
  — readers measured white labels inside dark bars at 2.56:1 and light faint text at 2.7–3.1:1.
- **OV-C200** With reduced motion set, nothing animates and nothing scrolls smoothly. *Why:* the
  reader's setting wins. *Regression:* no. *Source:* `assets/index.html:38`,
  `assets/index.html:346`.
- **OV-C201** Every row, button and disclosure that takes focus shows a visible focus ring. *Why:* a
  keyboard reader must see where they are. *Regression:* no. *Source:* `assets/index.html:64`.
- **OV-C202** On a phone the headline tiles sit two to a row with the largest across both, and
  side-by-side panels stack. *Why:* the same facts in a column. *Regression:* no. *Source:*
  `assets/index.html:137`, `assets/index.html:157`, `assets/index.html:351-352`.

### Hosted router

- **OV-C203** The page learns what the router can do from `/api/auth`'s `mode`, never assuming a
  Mac, and a router that predates the field is local with nothing changed. *Why:* the comment above
  `hosted()`. *Regression:* no. *Source:* `assets/index.html:2278-2282`,
  `assets/index.html:2507-2509`. *Under decision J:* the console ships inside the router it reads, so the older-router case here never reaches it; the rest of the rule stands, and the phones still meet older routers.
- **OV-C204** A hosted router hides the Claude Code setup section (login item, wrapper, route a
  project) and the network switch, and does not even ask for the setup state; a local router shows
  and reads both. *Why:* check "a hosted router hides what only a Mac has, and a local one keeps all
  of it". *Regression:* no. *Source:* `assets/index.html:602-616`, `assets/index.html:629-636`,
  `assets/index.html:2049-2051`, `assets/index.html:2300-2302`,
  `scripts/check-dashboard-ui.js:343-369`.
- **OV-C205** A hosted router adds an account by pasted token only, and the form opens on that
  choice; a local router offers sign-in (its default), token and profile. *Why:* "a hosted router
  never runs `claude`, so it can neither sign in from here nor keep a separate profile"; check "a
  hosted router adds an account by pasted token only, and a local one every way". *Regression:* no.
  *Source:* `assets/index.html:567-571`, `assets/index.html:2308-2318`,
  `scripts/check-dashboard-ui.js:371-383`.
- **OV-C206** A password the deployment pins reads "Set by the deployment. Change the Secret and
  restart the router." with nothing to set, change or remove, and a hosted router never asks for a
  first password, while a local one still asks on first boot at the machine. *Why:* check "a pinned
  password is the deployment's: said so, with nothing to set, change or remove". *Regression:* no.
  *Source:* `assets/index.html:2320-2326`, `assets/index.html:2433-2441`,
  `scripts/check-dashboard-ui.js:385-401`.
- **OV-C207** The connection snippet is the page's own origin, and carries `ANTHROPIC_AUTH_TOKEN`
  wherever the password is asked — always for a hosted router, a port-forward to localhost included,
  and for a local router read from another machine — and is the single line it always was at the
  machine itself. *Why:* check "the connection snippet is the page's own origin, with the password
  wherever it is asked". *Regression:* no. *Source:* `assets/index.html:955-966`,
  `scripts/check-dashboard-ui.js:403-425`.
- **OV-C208** A hosted router speaks of its deployment, not of this Mac: the store, push, password,
  session-key and token notes take hosted wording, the network chip says every caller must give the
  password, and an imported built-in account says a hosted router cannot sign with it; a local
  router gets its own words back exactly. *Why:* check "a hosted router speaks of its deployment,
  not of this Mac, and a local one as it always did". *Regression:* no. *Source:*
  `assets/index.html:2283-2306`, `assets/index.html:2040-2043`, `assets/index.html:2413`,
  `scripts/check-dashboard-ui.js:427-453`.
- **OV-C209** Settings open by themselves at most once per load, and only when nothing can sign: on
  a hosted router when no account it can sign with exists or a copied Claude Code login is the one
  in use, where an imported built-in account can be removed; on a local router also for its own
  broken built-in login, which it never offers to remove. *Why:* check "a hosted router opens the
  settings for a Claude Code login only while it is the one in use". *Regression:* no. *Source:*
  `assets/index.html:1966-1985`, `assets/index.html:2502`, `scripts/check-dashboard-ui.js:455-494`.
- **OV-C210** A hosted router's Router facts leave out the process id, its push-key hint names the
  deployment's push Secret, and a row left out never reaches the page as "null". *Why:* "A hosted
  router's process id is nothing anyone reading this can act on"; check "a hosted router's facts
  leave out its process id, and its key hint names the Secret". *Regression:* no. *Source:*
  `assets/index.html:979-988`, `assets/index.html:2455-2459`,
  `scripts/check-dashboard-ui.js:542-565`.

### Checks

- **OV-C211** The pure modules — formatters, forecast wording, scope model, search markup — load and
  run without a browser, and the contract specs call them directly. *Why:* check "script evaluates":
  nothing type-checks the page's script, and parsing it is what caught dead code in the first Spend
  card. *Regression:* yes — 04efe6b: "The dashboard script is parsed on every build now, which is
  how the dead code left in the first attempt at this card was caught." *Source:*
  `assets/index.html:652-720`, `scripts/check-dashboard-ui.js:143`.
- **OV-C212** No module-level name shadows a browser global the app uses: history, location, name,
  status, top, self, parent, origin, screen, event. *Why:* check "a local does not shadow a browser
  global the page uses". *Regression:* yes — 4a48211: a local named `history` stopped cards opening.
  *Source:* `assets/index.html:653-666`, `scripts/check-dashboard-ui.js:152-158`.

### Learned from Northstar's console

- **OV-C213** A chart is handed new figures only when one of them changes, and nothing while the
  tab is hidden; shown again, it draws the latest once. *Why:* Northstar's status chart was handed
  new objects every second. Each redraw queued d3 transitions that a hidden tab never runs, and
  the first frame back ran them all, leaving its dashboard blank for 21 s after five and a half
  minutes away. The activity chart polls every 5 s and would do the same.
  *Regression:* yes — Northstar `09321b4`. *Source:* Northstar `docs/13-console-contracts.md` §6,
  at `09321b4`.

## Sessions

The live view (`assets/live.html`, served at `/live`) becomes the Sessions screen. Line numbers
refer to the files as committed at `7965562` (`assets/live.html` last changed in `29c5eb3`). The web
page is the reference for both phones ([session liveness](session-liveness.md), "Client presentation
policy"), so a bullet that names phone tests is a three-client contract: it changes on all three
clients or not at all. Where a phone disagrees today, the bullet says so, and for Markdown the web's
semantics win (Decision F), but for lists, which follow the phones (SE-C99). A hosted router's list
is headed per machine, as the phones head it (SE-C219). Both were decided on 2026-10-05; on
2026-10-07 the console also tabbed the machines, which the phones do not (SE-C226). Where
today's page breaks a rule, the bullet's closing note says how; the rewrite keeps the rule, not the
defect.

### Session model

- **SE-C1** The router decides what a session is doing. The screen renders the router's `activity`,
  `tone` and `detail`, and derives no state of its own from the transcript, its timing or its
  silence. *Why:* four derivations once disagreed about one session: 99.9 % of generating minutes
  read idle, a running workflow read "Waiting for you", and a session that died mid-turn read
  "Working" for thirteen days. *Regression:* yes — 3542332. *Source:* `assets/live.html:481-488`,
  `docs/session-liveness.md:447-451`. *Phones:* iOS `testDotsTakeTheRoutersTone`; Android
  `the router's tone decides the colour, then the activity, then the old state`.
- **SE-C2** A session's tone is the router's `tone` when that is one of busy, attention, warn,
  error, idle or closed. Otherwise it is its `activity`'s tone: generating, tool and background are
  busy, waiting is attention, stalled is warn, failed is error, idle is idle, closed is closed. Only
  when it carries no `activity` at all is it its legacy `state`'s tone: working and tool are busy,
  ask is attention, stalled is warn, failed is error, idle is idle. So a `tool` row the router calls
  `warn` is warn, and a tone the screen does not know, beside a known activity, gives the activity's
  tone, never gray. *Why:* the router chooses the colour so that every client colours alike, `tone`
  is its choice when it makes one rather than something the screen cannot do without, and an older
  router's payload must read exactly as it always did. *Regression:* no. *Source:*
  `assets/live.html:489-501`; check
  `every activity and tone renders, and a word the page does not know is plain gray`
  (`scripts/check-live-ui.js:389-401`). *Phones:* iOS `testDotsTakeTheRoutersTone`; Android
  `the router's tone decides the colour, then the activity, then the old state`,
  `an activity this app does not know is neutral, never a crash and never a Stop` (an unknown tone
  defers to the activity). *Under decision J:* the console ships inside the router it reads, so the older-router case here never reaches it; the rest of the rule stands, and the phones still meet older routers.
- **SE-C3** An activity the screen does not know has no tone: the plain neutral dot, never the
  legacy `state`'s colour and never an error, while its words and duration still show. *Why:* a
  newer router saying something this screen has not heard of is not an error. *Regression:*
  no. *Source:* `assets/live.html:109-111, 486-488, 498`,
  `scripts/check-live-ui.js:260-261, 379-380`. *Phones:* iOS
  `testEveryRowSaysWhatItIsDoingAndSinceWhen` (an unknown activity still gets its line); Android
  `an activity this app does not know is neutral, never a crash and never a Stop`.
- **SE-C4** The tones wear the phones' palette in both themes: busy green, attention orange, warn
  amber, error red, idle gray, and closed a hollow gray ring. The `--st-*` values stay exactly as
  the phones have them (dark: busy `#9ece6a`, attention `#ff9e64`, idle `#8e8e93`; light: busy
  `#2c7a2c`, attention `#c2410c`, idle `#8e8e93`), and they stay separate from the accent colours of
  links, tabs and the question card. *Why:* a laptop and a phone read alike; the three clients'
  palettes were unified on 2026-09-29. *Regression:* no. *Source:*
  `assets/live.html:11-13, 20, 109-119`, `docs/session-liveness.md:472-475`. *Phones:* iOS
  `testDotsTakeTheRoutersTone`; Android
  `the router's tone decides the colour, then the activity, then the old state`.
- **SE-C5** One tone vocabulary serves every place a session is shown — its list row, its tab, its
  project's heading and the header's pip — so the header and the list cannot say the same thing in
  two colours. *Why:* two sets of classes are two chances to disagree. *Regression:* no. *Source:*
  `assets/live.html:55-67, 110-111`.
- **SE-C6** A busy dot pulses and an attention dot pulses faster; nothing else moves, and with
  reduced motion nothing pulses at all. *Why:* the pulse is the second cue after colour for what
  needs looking at, and reduced motion removes it, which is why the words must carry it too
  (SE-C20). *Regression:* no. *Source:* `assets/live.html:62-63, 70-71, 112-113, 121`.
- **SE-C7** A closed session, whose process has gone, is a hollow ring: a coloured border around an
  empty middle, because nothing is there to be idle. It can be read, but nothing typed will reach
  it. *Why:* closed and idle once read alike. *Regression:* yes — 3542332 (a closed session read
  idle). *Source:* `assets/live.html:67, 117-119`; check
  `a closed session is a hollow ring, says when it ended, and is never counted`
  (`scripts/check-live-ui.js:422-424`). *Phones:* iOS `testDotsTakeTheRoutersTone`; Android
  `a closed session is a hollow gray ring, never live and never stoppable`.
- **SE-C8** A session is live when it is busy — generating, tool, background or stalled, stalled
  being still busy because the process says it is working and only nothing has shown for a while —
  or when it is waiting on you. For a row whose activity the screen does not know, or that has none,
  live is the router's `active`. Live is what counts a session, keeps it out of "older", keeps it in
  the tabs and puts the halo on its dot. *Why:* one definition used everywhere; from an older router
  `active` is all a payload can say. *Regression:* no. *Source:*
  `assets/live.html:502-510`. *Phones:* iOS `testWorkingAndWaitingSessionsAreNeverFoldedAway`;
  Android `stop follows the activity when there is one, and the old state otherwise` (background is
  still live). *Under decision J:* the console ships inside the router it reads, so the older-router case here never reaches it; the rest of the rule stands, and the phones still meet older routers.
- **SE-C9** A live session's dot wears a halo in its list row and in its project's heading, whatever
  tone wins the colour; a tab's dot and the header's pip never do. *Why:* the halo still answers "is
  anything there busy or waiting" when a failure wins the colour. *Regression:* no. *Source:*
  `assets/live.html:120, 662, 689, 694, 1018`, `scripts/check-live-ui.js:673-676`. *Phones:* iOS
  `testTheProjectDotRanksTheSameEverywhere`; Android
  `the open tab follows its own stream, not the list row` (tabs never wear it).
- **SE-C10** A status is stoppable only when a turn is running: activity generating, tool or
  stalled, or, from a status with no activity, legacy state tool or working. Background work alone,
  waiting, idle, failed, closed and any activity the screen does not know are not stoppable. *Why:*
  with only background agents or workflows running there is no turn, and what an interrupt does to
  their undelivered results is unmeasured, so the router refuses it; a missing button beats one that
  stops the wrong thing. Background still reads `working` to an older client, and only the activity
  tells them apart. *Regression:* yes — 3542332 (Stop was offered, and refused, for background
  work). *Source:* `assets/live.html:511-518`; check
  `Stop is offered for a running turn only — never for background work alone`
  (`scripts/check-live-ui.js:720-742`). *Phones:* iOS `testStopIsOfferedOnlyForARunningTurn`;
  Android `stop follows the activity when there is one, and the old state otherwise`. *Under decision J:* the console ships inside the router it reads, so the older-router case here never reaches it; the rest of the rule stands, and the phones still meet older routers.
- **SE-C11** Every session from a current router has a row line, whatever it is doing, idle and
  failed included: the router's `detail`, then " · " and how long it has been at it since `since_ms`
  — "Waiting for you · 1h", "Waiting for you · background shell running · 10h", "API error: 529
  Overloaded · 1m". *Why:* how long one has sat waiting for you is worth a glance too; the phones
  had shown nothing for idle and ended sessions while the web showed both. *Regression:* yes
  — 3542332. *Source:* `assets/live.html:519-536, 669`, `docs/session-liveness.md:453-458`; check
  `each row says what it is doing and for how long, on the router's clock`
  (`scripts/check-live-ui.js:681-708`). *Phones:* iOS `testEveryRowSaysWhatItIsDoingAndSinceWhen`;
  Android `every row carries the router's line, idle and ended included`.
- **SE-C12** Durations are measured on the router's clock only: `since_ms` and `ended_ms` against
  the payload's own `as_of_ms`, never against the device's clock. A row without the router's clock
  gets no duration rather than a wrong one; a status without it falls back to the router's own
  `quiet_ms`, which is all an older router sent. *Why:* a laptop or phone whose clock is off would
  shift every duration; the checks set the router's clock seven minutes behind the test machine's,
  and the iPhone's tests eight days behind. *Regression:* no. *Source:*
  `assets/live.html:522-526, 530-534`, `scripts/check-live-ui.js:696-700, 714-717`. *Phones:* iOS
  `testEveryRowSaysWhatItIsDoingAndSinceWhen`, `testTheHeaderMeasuresEverythingOnTheRoutersClock`;
  Android `every row carries the router's line, idle and ended included`,
  `a row from a router that reads the process decodes every new key`. *Under decision J:* the console ships inside the router it reads, so the older-router case here never reaches it; the rest of the rule stands, and the phones still meet older routers.
- **SE-C13** A closed session's line says how long ago it ended — "Session ended · 3h ago", "Session
  ended mid-turn · 3h ago" — from `ended_ms` against `as_of_ms`; with either missing, the router's
  words alone. *Why:* for an ended session the useful time is how long it has been
  gone. *Regression:* no. *Source:* `assets/live.html:519-521, 531-533`; check
  `a closed session is a hollow ring, says when it ended, and is never counted`
  (`scripts/check-live-ui.js:411-420`). *Phones:* iOS `testEveryRowSaysWhatItIsDoingAndSinceWhen`,
  `testTheHeaderMeasuresEverythingOnTheRoutersClock`; Android
  `a closed session is a hollow gray ring, never live and never stoppable`.
- **SE-C14** The first four seconds go unsaid: a duration of four seconds or less is left off
  ("Thinking", not "Thinking · 2s"), and from just over four seconds it shows. *Why:* "· 2s" is a
  number that only flickers. *Regression:* no. *Source:* `assets/live.html:527-528, 535`,
  `scripts/check-live-ui.js:694-695`. *Phones:* iOS `testEveryRowSaysWhatItIsDoingAndSinceWhen`
  (four seconds without, five with); Android
  `every row carries the router's line, idle and ended included`.
- **SE-C15** A session with no `detail` has no row line at all, so an older router's rows, which
  carry none, look exactly as they did. *Why:* nothing said, nothing shown; a line made up from the
  legacy `state` would put new words on an old payload. *Regression:* no. *Source:*
  `assets/live.html:529`; check `a payload from an older router renders as it always did`
  (`scripts/check-live-ui.js:840-843`). *Phones:* iOS `testRowsAndDotsDrawForEveryActivity`; Android
  `every row carries the router's line, idle and ended included`,
  `a row from an older router reads exactly as it always did`. *Under decision J:* the console ships inside the router it reads, so the older-router case here never reaches it; the rest of the rule stands, and the phones still meet older routers.
- **SE-C16** A stretch of time is spelled the way it is glanced at: one unit, rounded — "12s", "5m",
  "3h", "2d" — and never negative. *Why:* a row line is read in passing. The phones spell two units
  ("3m 07s"); that is a known difference, not converged here. *Regression:* no. *Source:*
  `assets/live.html:459-465`. *Phones:* Android
  `elapsed time is spelled the way the iPhone spells it` (the phones' own spelling).
- **SE-C17** A payload from an older router, with no `activity` anywhere, renders exactly as it
  always did: dots from the legacy `state` (working with its halo, ask, failed, stalled, idle, and
  an unknown word plain gray), no row lines, and liveness from `active`. *Why:* installed phones and
  older routers stay in service, and every key the router added is optional. *Regression:*
  no. *Source:* `assets/live.html:484-487, 506-510, 543-547`; check
  `a payload from an older router renders as it always did`
  (`scripts/check-live-ui.js:825-875`). *Phones:* iOS `testDotsTakeTheRoutersTone`,
  `testRowsAndDotsDrawForEveryActivity`; Android
  `a row from an older router reads exactly as it always did`. *Under decision J:* the console ships inside the router it reads, so the older-router case here never reaches it; the rest of the rule stands, and the phones still meet older routers.
- **SE-C18** A project heading's dot is its most pressing session: attention (any waiting) over
  error (any failed) over warn (any stalled) over busy (any generating, tool or background) over
  idle (any open and idle), and the hollow ring only when every session in it has ended. An activity
  the screen does not know outranks nothing and has not ended, so beside an idle session it gives
  idle, and beside only ended ones neither colour nor ring. *Why:* each of the first three is a
  thing to go and look at. The three clients had ranked three ways: a project of ended sessions drew
  a ring on the web, a gray dot on the iPhone and nothing on Android, and a failed session never
  reached the heading. *Regression:* yes — 3542332; check
  `a project heading shows its most pressing session, ranked as on the phones`. *Source:*
  `assets/live.html:537-551`, `docs/session-liveness.md:459-462`,
  `scripts/check-live-ui.js:629-679`. *Phones:* iOS `testTheProjectDotRanksTheSameEverywhere`;
  Android `a project's heading is ranked the same on every client`.
- **SE-C19** When none of a project's rows carries an activity (an older router), its heading says
  what it always did: busy if any row is `active`, else idle. *Why:* such a router can only say that
  something moved lately. *Regression:* no. *Source:* `assets/live.html:543-547`,
  `scripts/check-live-ui.js:844-848`. *Phones:* iOS `testTheProjectDotRanksTheSameEverywhere`;
  Android `a project's heading is ranked the same on every client` (Android draws no dot for such a
  project with nothing live: each client keeps its previous behaviour with an older router). *Under decision J:* the console ships inside the router it reads, so the older-router case here never reaches it; the rest of the rule stands, and the phones still meet older routers.
- **SE-C20** No dot carries its meaning by colour alone. Every dot — row, heading, tab and header —
  has its words as visible text or as its accessible name, in the phones' words: "working", "running
  a tool", "waiting for you", "stalled", "failed", "idle", "ended". *Why:* under deuteranopia
  "waiting on you" and "stalled" are ΔE 2–4 apart, by the plan's readers' measure, and the pulse,
  the only other cue, goes with reduced motion. *Regression:* no. *Source:*
  `assets/live.html:109-121, 694, 1017-1018`. *Phones:* iOS `testDotsTakeTheRoutersTone`,
  `testTheProjectDotRanksTheSameEverywhere` (every look carries its label). *Today:* broken — a
  heading's dot and a tab's dot are bare colour, and a tab's words exist only in its hover title.
- **SE-C21** Nothing drawn from a row or a status reads "undefined", "NaN" or "null", for any state
  either router can send or a word neither has sent yet. *Why:* a word that slips through as
  "undefined" reads as a broken router. *Regression:* no. *Source:*
  `assets/live.html:556-566, 657-670`; check `the session list renders every state`
  (`scripts/check-live-ui.js:298-316`).
- **SE-C22** A row is drawn from whatever keys it has, and one malformed or minimal row never blanks
  or truncates the list. *Why:* one bad row would otherwise hide every good one after
  it. *Regression:* no. *Source:* `assets/live.html:657-673, 677-699`. *Phones:* Android
  `a row from an older router reads exactly as it always did` (a row of `session_id` alone
  decodes). *Today:* broken — a row without `session_id` throws inside the render, and every row
  after it goes undrawn.

### List

- **SE-C23** The list groups sessions by project, the project being the last segment of the
  session's `cwd`, split on `/` and `\` alike (SE-C222), else the router's `project`. On a hosted
  router that lists more than one machine, it groups them by machine first, and by project within
  each machine (SE-C219). *Why:* the list is mostly "which of my projects is moving". *Regression:*
  no. *Source:* `assets/live.html:139, 479, 675-683`. *Phones:* iOS
  `testSessionsGroupLikeTheWebView`; Android `sessions group like the web view`.
- **SE-C24** Projects are in alphabetical order, so a project keeps its place instead of jumping to
  the top whenever one of its sessions moves; on a list headed per machine, each machine's projects
  are in that order under its heading (SE-C219). Within a project, sessions are newest first by
  `last_activity_ms`. *Why:* a list that reshuffles every five seconds cannot be read, and within
  one project recency is what you are choosing between. The comment at 675-676, that the project
  touched last leads, is stale. *Regression:* yes — bbe0780. *Source:*
  `assets/live.html:684-688`. *Phones:* iOS `testSessionsGroupLikeTheWebView`; Android
  `sessions group like the web view`.
- **SE-C25** Each project heading shows its dot (SE-C18), its name, how many sessions it holds and
  the age of its newest session. *Why:* whether a project is moving is answered without opening
  it. *Regression:* no. *Source:* `assets/live.html:140-145, 689-697`.
- **SE-C26** A session's row shows its dot, with the halo when live; its name, which is the git
  branch or the session id's first eight characters when the session has a title, and the project
  otherwise; whether it is live or view-only (SE-C28); its age; its title, in at most two lines; its
  row line (SE-C11); and the session id's first eight characters with its branch. *Why:* the row
  sits under its project's heading, so it names the branch or the id rather than repeating the
  project. *Regression:* no. *Source:* `assets/live.html:122-137, 657-670`.
- **SE-C27** A row's words are coloured only when they need a reader — waiting (attention), stalled
  (warn) and failed (error). Busy, idle and closed lines stay neutral, and the dot carries the
  rest. *Why:* colour is kept for what to go and look at. *Regression:* no. *Source:*
  `assets/live.html:127-133, 669`, `scripts/check-live-ui.js:387-388`.
- **SE-C28** Each row says whether its session can be typed into: "live" (wrapped, so it takes a
  typed message and a Stop) or "view" (read-only until it is started after installing the
  wrapper). *Why:* whether a session is wrapped decides whether the composer and Stop can do
  anything, and finding out by sending and reading the warning afterwards is worse. *Regression:*
  no. *Source:* `assets/live.html:134-137, 664-666`.
- **SE-C29** A session quiet for more than twelve hours folds into an "older" group, grouped by
  project in the same way, but a live one never does: a session waiting on you writes nothing while
  it waits, so it would fold away at exactly the moment it most needs finding. Closed, idle and
  failed sessions fold like any other. On a list headed per machine, each machine has an older group
  of its own, under its heading (SE-C219). *Why:* the list is for what you are working on
  now. *Regression:* yes — check `a session busy or waiting on you is never folded into "older"` (a
  question that had waited twelve hours folded away on all three clients). *Source:*
  `assets/live.html:138, 702-710`, `scripts/check-live-ui.js:440-456`. *Phones:* iOS
  `testWorkingAndWaitingSessionsAreNeverFoldedAway`; Android
  `a session waiting on you stays recent however long it has waited`.
- **SE-C30** The older group says how many it holds: "1 older session", "2 older sessions". *Why:* a
  closed group must still say what it hides. *Regression:* no. *Source:* `assets/live.html:719`,
  `scripts/check-live-ui.js:452`.
- **SE-C31** The older group keeps its open or closed state across every refresh, every archive and
  every return to the list. On a list headed per machine, each machine's older group opens and
  closes on its own, and keeps its own state. *Why:* a section the reader opened must not snap shut
  under them. *Regression:* yes — f46fcbc (the fold was built to keep its state). *Source:*
  `assets/live.html:647, 712-718`. *Today:* broken — the list is emptied before the old group's
  state is read, so it is never found, and the fold closes on every five-second refresh.
- **SE-C32** The archived shelf is visible, not folded behind a disclosure: it says how many it
  holds ("3 archived") and lists its sessions flat, newest first, each dimmed. On a list headed per
  machine, each machine has a shelf of its own, under its heading (SE-C219). *Why:* Decision I;
  controls stay visible rather than behind disclosure. *Regression:* no. *Source:*
  `assets/live.html:146-152, 365, 725-744`. *Phones:* iOS `testSessionsGroupLikeTheWebView`; Android
  `sessions group like the web view` ("on the shelf however recently it moved, newest
  first"). *Today:* broken — the shelf is a closed `<details>`, and like the older group it snaps
  shut on every refresh.
- **SE-C33** A session the router marks `archived` is on the shelf only, never also among the recent
  or older groups, however recently it moved. The router decides `archived` — "put away, and quiet
  ever since" — and clears it itself once the session shows life, so the screen takes it as given
  and never re-derives it from a time window. *Why:* putting something away says "I am done with
  this", not "never show me this"; tested against a window, archiving hid nothing for half a day,
  because a session moves a moment before it is put away. *Regression:* yes — e784bcd. *Source:*
  `assets/live.html:596-602, 606, 703-704`. *Phones:* iOS `testSessionsGroupLikeTheWebView`; Android
  `sessions group like the web view`.
- **SE-C34** Every row carries a visible Archive action, and every shelved row a visible Restore
  action, both with the Archive concept icon. The row itself opens its session, and its action never
  does; Restore brings a session back to the list for good. Archiving or restoring keeps the list
  where it was. *Why:* Decision I: archiving is a visible action on every row and tab, one icon for
  one concept. *Regression:* no. *Source:* `assets/live.html:735-740, 1030-1048`. *Today:* broken —
  a list row cannot be archived at all, only a tab can, and Restore is a span styled as a link.
- **SE-C35** With no session open, the header counts what is at work and what waits on you: "4
  working · 1 waiting", working being generating, tool, background or stalled, and waiting being
  waiting. Either part is left out at zero; the pip is attention when anything waits, else busy when
  anything works. A closed, idle or failed session is never counted. *Why:* "7 active" could not
  tell a session blocked on you from one at work. *Regression:* yes — 3542332 (the drift recorded in
  Android `the router says how many sessions work and how many wait`). *Source:*
  `assets/live.html:633-640`, `docs/session-liveness.md:470-471`,
  `scripts/check-live-ui.js:406-408, 421`. *Phones:* iOS `testCountsSayWorkingAndWaiting`; Android
  `the router says how many sessions work and how many wait`.
- **SE-C36** With nothing at work or waiting, the header says how long since anything last moved —
  "idle · 5m" — or "nothing yet" when there are no rows, under the idle pip. *Why:* a quiet router
  still answers "when did anything last happen". *Regression:* no. *Source:*
  `assets/live.html:631-632, 638-640`, `scripts/check-live-ui.js:430`.
- **SE-C37** From an older router, when no row carries an activity, the header says what it always
  did: "N active" from `active`, busy when N is not zero, else the idle line. *Why:* `active` is all
  such a router can say. *Regression:* no. *Source:* `assets/live.html:641-646`,
  `scripts/check-live-ui.js:844-846`. *Phones:* iOS `testCountsSayWorkingAndWaiting` ("2 active"). *Under decision J:* the console ships inside the router it reads, so the older-router case here never reaches it; the rest of the rule stands, and the phones still meet older routers.
- **SE-C38** An empty list says which empty it is: from a local router "No sessions in the last
  week.", and from a hosted router why it lists none (SE-C187). *Why:* an empty list and a router
  that cannot have sessions are different facts. *Regression:* no. *Source:*
  `assets/live.html:647-648`; check
  `a hosted router says why its list is empty, and warns of no wrapper`
  (`scripts/check-live-ui.js:1103-1111`).
- **SE-C39** A local router warns above the list when the newest editor extension is not wrapped —
  "<extension> is not wrapped — sessions it starts are view-only until the wrapper is installed" —
  and points to the Settings that install it. *Why:* an extension update ships a fresh, unwrapped
  binary, and every session it starts is view-only with nothing to say so; the list is where the
  reader will be wondering. *Regression:* no. *Source:* `assets/live.html:624-630, 649-655`,
  `scripts/check-live-ui.js:1099-1105`. *Phones:* iOS `testStreamItemsDecode` (the newest unwrapped
  extension).
- **SE-C40** The list is read again every five seconds, on the app's one clock, and whenever it is
  shown again; a refresh, an archive and a restore keep the reader's scroll position. *Why:* the
  list is a live index, and losing your place to a refresh makes it unreadable (Northstar `docs/13`
  §6). *Regression:* no. *Source:* `assets/live.html:1044-1046, 1395-1403, 1691-1696`.
- **SE-C41** A list read that fails while no session is open says so in the header — "offline ·
  <reason>" — and keeps the rows it last had, marked with their age, never blanked and never read as
  an empty list. *Why:* losing what you were reading to a blip is worse than seeing it stale and
  marked (Northstar `docs/13` §1). *Regression:* no. *Source:* `assets/live.html:612-617`,
  `src/live.rs:1217-1223`. *Today:* broken — the old rows stay up with nothing marking them old, and
  a reply that is not a list (the router's 500 `{ok: false, reason}`) is taken for the rows and
  throws inside the refresh.
- **SE-C42** With a session open, a failed list read says nothing over the header, which belongs to
  that session's stream. *Why:* the list's trouble is not the session's status. *Regression:*
  no. *Source:* `assets/live.html:607-610, 614-616`; check
  `a dropped stream leaves the header stale, not showing its last word`
  (`scripts/check-live-ui.js:505-509`).
- **SE-C43** Every age on the list — a row's, a heading's, the header's "idle · N" — and the
  twelve-hour windows that fold sessions and admit tabs are measured on the router's clock
  (`as_of_ms`) when the rows carry it. *Why:* the row line refuses the device's clock because a
  clock that is off shifts every duration (SE-C12), and an age is a duration too. *Regression:*
  no. *Source:* `assets/live.html:465, 603-605, 631-632, 667, 696, 702-708`. *Today:* broken — the
  ages, `isCurrent` and the older fold all read the device's `Date.now()`.

### Tabs

- **SE-C44** With a session open, a strip over the feed holds its project's sessions on its machine
  (SE-C221): the open one, and every sibling that is current and not archived. Current means live,
  an older router's working or ask, or active in the last twelve hours. *Why:* a strip of every
  session the project ever had is a worse index than the list it came from. *Regression:* yes —
  3918cc5. *Source:*
  `assets/live.html:603-605, 995-1002`; check `stale sessions do not clutter the tabs`
  (`scripts/check-live-ui.js:352-359`). *Phones:* iOS
  `testWorkingAndWaitingSessionsAreNeverFoldedAway` (an ended session is not a tab; an older
  router's ask is). *Under decision J:* the console ships inside the router it reads, so the older-router case here never reaches it; the rest of the rule stands, and the phones still meet older routers.
- **SE-C45** The open session always has its tab, archived, stale or ended, so a tab never
  disappears from under the reader. *Why:* the strip must hold what you are looking
  at. *Regression:* no. *Source:* `assets/live.html:1000-1002`.
- **SE-C46** Tabs keep creation order — `created_ms`, else `last_activity_ms` — oldest first, so a
  tab keeps its position for the life of its session. *Why:* ordering by activity moves a tab out
  from under the cursor mid-click, and a strip that rearranges itself every five seconds cannot be
  navigated. *Regression:* yes — fd95731; check
  `tabs cover the project and keep creation order`. *Source:* `assets/live.html:350-351, 1003-1005`,
  `scripts/check-live-ui.js:318-325`.
- **SE-C47** A project with fewer than two such sessions shows no strip at all, never a strip of one
  tab. *Why:* one tab is no choice. *Regression:* yes — the checks that first asserted one tab each
  could never fail, so neither was ever seen to be wrong
  (`scripts/check-live-ui.js:327-330`). *Source:* `assets/live.html:1007-1009`; check
  `stale sessions do not clutter the tabs` (`scripts/check-live-ui.js:360-364`).
- **SE-C48** An archived sibling leaves the strip, and comes back by itself once the router clears
  its `archived` because it woke. *Why:* the router's word on `archived` is taken as given
  (SE-C33). *Regression:* yes — e784bcd; check
  `archived sessions leave the tabs, but come back when live`. *Source:* `assets/live.html:1002`,
  `scripts/check-live-ui.js:334-350`.
- **SE-C49** Each tab shows its dot and its session's title, else the session id's first eight
  characters, cut short with an ellipsis; choosing a tab opens that session, and the open tab is
  marked. *Why:* the strip is how a project's sessions are switched between. *Regression:*
  no. *Source:* `assets/live.html:355-360, 1011-1024`.
- **SE-C50** Every tab carries a visible Archive action with the Archive icon, at every width and
  with any pointer, sized as a target of its own, and a tap on the tab is never taken for one on the
  action. Archiving the open session keeps its tab for as long as it stays open. *Why:* Decision I;
  Carbon's dismissable Tabs hard-code a Close glyph, which says "dismiss", not "put
  away". *Regression:* no. *Source:* `assets/live.html:362-364, 1020-1022, 1042`. *Today:* broken —
  the put-away mark is a ✕ at zero opacity except on hover or on the open tab, so on a phone it is
  invisible yet still takes taps near a tab's edge.
- **SE-C51** The open tab's dot says what the stream says while the stream vouches for anything,
  changing the moment the stream's word changes rather than at the next list refresh. While the
  stream does not vouch (connecting, reconnecting, silent) it falls back to the list's row. Every
  other tab is its list row, at most five seconds old. *Why:* the header and the tab beside it must
  not disagree, as when a tab stayed gray while the header said the turn was running. *Regression:*
  yes — 3542332; check `the tab dots follow the list while a session is open`. *Source:*
  `assets/live.html:553-555, 584-593, 1012-1014`, `docs/session-liveness.md:468-469`,
  `scripts/check-live-ui.js:464-468, 552-558`. *Phones:* iOS `testTheOpenTabFollowsTheStream`;
  Android `the open tab follows its own stream, not the list row` (Android turns a stale open tab
  neutral gray instead of falling back to its row; the web's fallback is the rule, pinned at
  `scripts/check-live-ui.js:557-558`).
- **SE-C52** While a session is open the list keeps refreshing, and each refresh redraws the strip,
  so a sibling that starts working or asking shows it without the reader leaving; that refresh never
  touches the header, which belongs to the open session's stream. *Why:* the strip once kept the
  states it had when the session was opened. *Regression:* yes — check
  `the tab dots follow the list while a session is open`
  (`scripts/check-live-ui.js:470-477`). *Source:* `assets/live.html:607-610, 618-622, 1691-1692`.
- **SE-C53** While Find covers the session, refreshes do not draw the strip over it, and closing
  Find redraws it current, which may mean no strip at all. *Why:* the refreshes skipped the hidden
  strip, and a project now down to one current session has none to show. *Regression:* no. *Source:*
  `assets/live.html:620-621, 1450-1452`, `scripts/check-live-ui.js:479-485, 1007-1010`.
- **SE-C54** The strip is a single row that scrolls sideways and never wraps, and it brings the open
  tab into view. *Why:* wrapped tabs cannot be reached on a phone; Carbon's tabs are capped at one
  64 px row (Northstar `29472eb`). *Regression:* no. *Source:* `assets/live.html:352-354, 1027`.

### Stream

- **SE-C55** Opening a session connects to its stream, naming the session by its real id,
  URL-encoded (`/api/live/stream?session=<id>`). With only the list on screen no stream is open and
  nothing is watched. *Why:* one connection per session on screen; a phone once asked for a session
  literally named `${session.sessionId}` and never drew its prompt card. *Regression:* yes — 3542332
  (Android RC9). *Source:* `assets/live.html:1069-1074, 1125-1126`,
  `scripts/check-live-ui.js:526-529, 599-604`. *Phones:* Android
  `the prompt is asked for by the session's real id`,
  `no string in the app escapes its own template`.
- **SE-C56** Only one connection is listened to at a time: opening a new one closes the old, and
  anything a replaced connection still delivers — a row, a status, an error — changes nothing. The
  stream hook keeps this through React StrictMode's double mount. *Why:* a replaced connection is
  history, whatever it still delivers. *Regression:* no. *Source:*
  `assets/live.html:1070, 1075-1082, 1106`, `scripts/check-live-ui.js:572-575`; plan, "Data".
- **SE-C57** Leaving the session closes its connection at once, not when a timer would. *Why:* a
  page that has moved on must not hold the socket, and a retry must not wait behind a dead
  read. *Regression:* no. *Source:* `assets/live.html:1388`,
  `scripts/check-live-ui.js:599-604`. *Phones:* iOS `testLeavingThePageEndsTheStreamPromptly`;
  Android `a collector that stops closes the connection at once, even a silent one` (EventsTest.kt).
- **SE-C58** The stream's events are a closed set, each registered by its name: an unnamed event is
  a feed row, `status` is what the session is doing, and `error` is the router refusing the
  stream. *Why:* only registered names dispatch (Northstar `bcd16e2`, `docs/13` §6). *Regression:*
  no. *Source:* `assets/live.html:1083-1109`, `src/live.rs:2231-2233, 2281, 2303`.
- **SE-C59** The router's own `error` event ("no such session") is the router speaking: its words
  are shown, and it is never taken for a dropped connection nor retried every ten seconds. *Why:* a
  stream that will never deliver must not read "reconnecting…" for the rest of the
  visit. *Regression:* no. *Source:* `assets/live.html:1103-1109`, `src/live.rs:2231-2233`,
  `android/app/src/main/java/ca/whnet/agentrouter/feed/LiveFeed.kt:246`. *Today:* broken — the
  listener for the connection's own `error` also receives the router's named event, so the header
  reads "reconnecting…" forever, the reason is never shown, and the session is retried for as long
  as it stays open.
- **SE-C60** A stream the router refuses to the caller — 401, a wrong or missing password — is said
  as such and not retried. *Why:* a wrong password will not come right by retrying, and a refused
  stream must not look like a reconnecting one (Northstar `docs/13` §6). *Regression:* no. *Source:*
  `assets/live.html:1103-1109, 1122-1133`. *Phones:* Android
  `a refused password is said as such, so the feed stops retrying` (EventsTest.kt). *Today:* broken
  — `EventSource` reports no status, so the page reads a refusal as "reconnecting…" and retries
  every ten seconds for as long as the session is open.
- **SE-C61** Until a connection's first status arrives, the header says "connecting…" with a neutral
  pip and Stop hidden — never the list's summary as though it were this session's status. Rows that
  arrive before the first status do not vouch for one. *Why:* until its stream speaks, the header
  has nothing true to say about this session. *Regression:* yes — 3542332; check
  `a dropped stream leaves the header stale, not showing its last word` ("the list summary was left
  as the session status"). *Source:* `assets/live.html:576-582, 1157-1159`,
  `scripts/check-live-ui.js:488-496`. *Phones:* iOS `testAClosedStreamLeavesTheHeaderReconnecting`
  ("a row is not a status"); Android
  `the header shows a status only while the stream vouches for it`.
- **SE-C62** While the stream vouches for it, the header shows the latest status: its line as a
  row's (SE-C11), then " · N queued" when N > 0 messages wait in the session's own queue, beside a
  pip in the status's tone. *Why:* the header is the session's present, and a queued message is part
  of it. *Regression:* no. *Source:* `assets/live.html:556-566`,
  `scripts/check-live-ui.js:498-499`. *Phones:* iOS
  `testTheHeaderMeasuresEverythingOnTheRoutersClock`; Android
  `the header line says how long and how many are queued`.
- **SE-C63** A status always says something: one whose `detail` is empty shows its state's
  word. *Why:* a blank header reads as a broken one; the replay check counts a header that "says
  nothing" as a failure. *Regression:* no. *Source:* `assets/live.html:529, 564-566`,
  `scripts/check-live-ui.js:932`. *Phones:* Android
  `the header line says how long and how many are queued` ("idle"). *Today:* broken — a status with
  an empty `detail`, as an older router sends for idle, leaves the header blank.
- **SE-C64** When the connection errors, the header stops vouching at once: "reconnecting…" in
  neutral italics, the pip gray, Stop gone, and the open tab back to its list row. The first status
  the browser's own retry brings is believed again. *Why:* what the stream last said is no longer
  known to be true, and a Stop aimed at a picture that may be minutes old could land on a turn
  nobody has seen. *Regression:* yes — check
  `a dropped stream leaves the header stale, not showing its last word` ("Stop left up on a status
  nobody can vouch for"). *Source:* `assets/live.html:68-69, 572-582, 1103-1109`,
  `scripts/check-live-ui.js:502-513`. *Phones:* iOS `testAClosedStreamLeavesTheHeaderReconnecting`,
  `testASilentStreamStopsVouchingForItsLastStatus`; Android
  `the header shows a status only while the stream vouches for it`.
- **SE-C65** An error while the connection is still CONNECTING means the browser is reconnecting, so
  the screen never shows a live status over a link it cannot vouch for; once the browser has given
  up (CLOSED), the screen's own watchdog reconnects, unless the router refused the caller
  (SE-C60). *Why:* checking only for CLOSED showed "live" over dead data (Northstar
  `1374e53`). *Regression:* no. *Source:* `assets/live.html:1103-1109, 1122-1133`; plan, "Data".
- **SE-C66** Ten seconds with nothing on the stream — no row and no status — means the connection
  died without a word. The header turns "reconnecting…", Stop goes, the open tab returns to its row,
  and the silent connection is closed and a new one opened. *Why:* the router speaks at least once a
  second whatever the session is doing, even for an ended one, so ten seconds of nothing is a NAT
  that forgot the link, a Mac that slept or a network that went away — which a browser can take
  minutes to notice, while the header goes on saying "Running Bash · 12s" with Stop beside
  it. *Regression:* yes — 3542332; check
  `a stream silent for ten seconds is taken for dead: stale, no Stop, and reconnected`. *Source:*
  `assets/live.html:1050-1058, 1122-1133`, `docs/session-liveness.md:463-467`,
  `scripts/check-live-ui.js:516-560`. *Phones:* iOS
  `testSilenceMeansTenSecondsAndAReconnectRebuildsTheFeed`,
  `testASilentStreamStopsVouchingForItsLastStatus`; Android
  `a silent stream is given up on after ten seconds, as on every client`,
  `a feed whose stream falls silent stops vouching for its status and reconnects` (EventsTest.kt).
- **SE-C67** Every row and every status starts the ten seconds again, and each connection gets ten
  seconds of its own from when it opens, so one that never says anything (the router down) is
  retried every ten seconds, not every second, with the rows still there to read. *Why:* a storm of
  reconnects helps nobody, and the reader keeps what they were reading. *Regression:* no. *Source:*
  `assets/live.html:1071, 1077-1082, 1122-1133`, `scripts/check-live-ui.js:540-551, 577-589`.
- **SE-C68** A clock set back never reads as a silence that has not begun, nor holds the next
  reconnect off by however far it moved: silence is measured from no later than now. *Why:* a clock
  being corrected by an hour must not postpone a reconnect by an hour. *Regression:* no. *Source:*
  `assets/live.html:1128-1129`, `scripts/check-live-ui.js:591-597`.
- **SE-C69** The router cannot resume a stream: every connection, the browser's own retry included,
  begins by replaying the transcript's tail. A connection that follows another therefore starts a
  new copy of the feed, but only when it delivers its first event, not when it opens, so what was
  read stays on screen while the link is down; the tail lands once, never twice. *Why:* a feed that
  appended the replay would show every row twice after any hiccup. *Regression:* yes — check
  `the browser reconnecting by itself does not show the tail twice`. *Source:*
  `assets/live.html:1059-1068, 1077-1082, 1112-1120`, `docs/session-liveness.md:463-466`,
  `scripts/check-live-ui.js:562-568, 607-627`. *Phones:* iOS
  `testSilenceMeansTenSecondsAndAReconnectRebuildsTheFeed`; Android
  `a reconnect after a silence does not append the backfill twice` (EventsTest.kt).
- **SE-C70** The first connection after a session opens does not start a new copy when it speaks, so
  what was echoed before it spoke (a message sent at once, on a feed just emptied) stays. Android
  clears on every connection's first event, a known difference. *Why:* nothing has been read twice
  yet, and the echo is the only sign the send worked. *Regression:* no. *Source:*
  `assets/live.html:1072, 1080`, `scripts/check-live-ui.js:617-621`,
  `android/app/src/main/java/ca/whnet/agentrouter/feed/LiveFeed.kt:236-239`.
- **SE-C71** A new copy of the feed rebuilds the ArrowUp history from the replay and resumes
  following the newest row. *Why:* the history is what the transcript says was sent, and the replay
  is the transcript. *Regression:* no. *Source:* `assets/live.html:1112-1120`,
  `scripts/check-live-ui.js:626`.
- **SE-C72** A row that cannot be parsed or drawn is skipped, and the rows after it still land; in
  the tests the same row is a failure. *Why:* one bad row must not stop the feed, and a row that
  silently did not land must not pass a test. *Regression:* no. *Source:*
  `assets/live.html:1085-1097`, `scripts/check-live-ui.js:877-921`.
- **SE-C73** Feed rows appear in the order the stream delivers them, and the screen never re-sorts
  them by any timestamp. *Why:* the transcript is the conversation's order — a result can precede
  its own call, and compaction re-appends lines stamped in the past — so a feed sorted by a clock
  would reorder it (Northstar `docs/13` §10). *Regression:* no. *Source:*
  `assets/live.html:1093-1094`, `docs/session-liveness.md:201-208`.
- **SE-C74** When the page becomes visible again after being hidden (a browser tab in the
  background, a phone switched to another app), it reads the list again at once and, with a session
  open, reconnects at once if the stream went quiet meanwhile. *Why:* a phone backgrounds the page
  for minutes at a time (Northstar `docs/13` §6). *Regression:* no. *Source:*
  `assets/live.html:1122-1133, 1691-1696`.

### Feed

- **SE-C75** The feed reads as a conversation, not a log. The person's own words are the only filled
  bubble, right-aligned and at most 85 % of the column or 40em wide; assistant prose sits on the
  page with no chrome, at most 46em wide; tools, thinking and the rest recede onto a thin
  rail. *Why:* "what did I ask?" is what you scan for, the answer is what you read, and the rest is
  apparatus. *Regression:* no. *Source:* `assets/live.html:159-163, 177-199`.
- **SE-C76** A row is classified by its kind before its role: only a user `text` row is the person's
  bubble, a tool result, which the transcript writes under the user role, is never drawn as their
  words, and an unknown kind falls back on its role. *Why:* dispatching on the role rendered every
  command's output as something the person had typed. *Regression:* yes — Android `FeedRowKindTest`
  (found reading a real feed). *Source:* `assets/live.html:958-964, 986-991`. *Phones:* Android
  `a tool result is not the person talking`.
- **SE-C77** A cancellation — "[Request interrupted by user]" and its siblings, kind `interrupt` —
  is a centred, faint, italic note with no speaker. *Why:* a cancellation is something that
  happened, not something anyone said, and drawn as a request it misattributes it. *Regression:* yes
  — 35d1230, 9cbe4fc (cancellations stored as a text block were still blue bubbles). *Source:*
  `assets/live.html:254-258`.
- **SE-C78** The person's words, answers, slash commands and other plain rows are shown as written,
  whitespace kept and never as Markdown; only assistant prose and side answers are Markdown. *Why:*
  Markdown is how the model writes, not how the person typed. *Regression:* no. *Source:*
  `assets/live.html:984-991`.
- **SE-C79** A tool call is a terminal block: the tool's name and duration over a raised command
  line, and only Bash gets the "$" prompt glyph. *Why:* a file path or a search query was never
  typed at a prompt, and drawing one there would be a small lie about what happened. *Regression:*
  no. *Source:* `assets/live.html:199, 201-213, 970-975`.
- **SE-C80** A tool's result hangs directly off its call in the same block style, under a small OUT
  label, with no gap to read across, and an empty output adds nothing. *Why:* a command and its
  output are one thing; two rows of equal weight left the reader working out which output belonged
  to which command. *Regression:* no. *Source:* `assets/live.html:201-203, 214-218, 976-978`.
- **SE-C81** A command line never wraps — it scrolls sideways inside its block — while output wraps
  anywhere. *Why:* a wrapped path or flag is harder to read than a scrolled one, while prose and
  stack traces want the full width. *Regression:* no. *Source:* `assets/live.html:205-208, 214-215`.
- **SE-C82** Thinking collapses to "Thought for 2s ›" ("Thought ›" at 400 ms or less), opens and
  closes on demand, and is never clamped. *Why:* thinking took as much room as the answer and pushed
  it off a phone's screen; the real client collapses it too. *Regression:* no. *Source:*
  `assets/live.html:219-221, 979-983`.
- **SE-C83** A subagent's rows sit on a warn-coloured rail and are labelled "subagent"; an answer
  row is "✓ answered" on an ok-coloured rail; a slash command is "⌘ command"; a side answer is "side
  answer". *Why:* each is told apart at a glance from the main conversation. *Regression:*
  no. *Source:* `assets/live.html:222, 253, 259, 963-964, 984-985, 989`.
- **SE-C84** A row's duration shows only above 400 ms: "820ms" under a second, "2.8s" under ten
  seconds, "18s" under a minute, else "3m". *Why:* a duration worth reading is one that took a
  while. *Regression:* no. *Source:* `assets/live.html:456-458, 965, 980`.
- **SE-C85** Anything tall folds behind a toggle and is never cut: prose and plain rows at 11em,
  terminal blocks at four lines. The toggle says how many lines it hides ("Show 42 lines") and
  becomes "Show less", and it appears only when the content really overflows. *Why:* clipping makes
  the rest unreachable, which is why the router's cap went from 1,200 to 6,000 characters; four
  lines are enough to recognise a command or see the head of its output. *Regression:* yes — b124c9a
  (folding had never once fired), 661bbc5. *Source:* `assets/live.html:312-324, 796-819`.
- **SE-C86** The fold clamps first and measures second, and unclamps when the content fits within
  four pixels. *Why:* an unclamped element always reports its scroll height equal to its height, so
  measuring first never folded anything. *Regression:* yes — b124c9a. *Source:*
  `assets/live.html:800-807`; plan ("a `Clamp` that clamps before it measures").
- **SE-C87** Following the newest row is a state, not a test made at each append. It is on while the
  reader is within 140 px of the bottom, set by their own scrolling, and while it is on, any change
  in a row's height — a fold a frame after the row lands — pins the view again, at once and once
  more after layout settles. *Why:* rows change height after they are appended, so a scroll issued
  at append time lands short and the view silently stops following; and the scroller itself never
  resizes, so the rows are what is observed. *Regression:* yes — 570e6c6, c6baf82. *Source:*
  `assets/live.html:426-427, 447-450, 1097, 1165-1177`.
- **SE-C88** Following resumes when a session opens, when the feed is rebuilt, and when the reader
  sends a message or a side question. *Why:* what the reader just did is at the
  bottom. *Regression:* no. *Source:* `assets/live.html:1117, 1169, 1606-1607, 1646`.
- **SE-C89** Nothing in the feed widens it: every grid item may shrink, transcripts, output, paths
  and links wrap anywhere, and only a command line, a fenced block or a table scrolls sideways,
  inside its own box. *Why:* a grid item defaults to `min-width: auto`, so one wide `<pre>`
  stretched the column and the whole page scrolled sideways; one unbroken URL widened a 390 px page
  to 1,119 px (Northstar `4b65e8b`). *Regression:* yes — b124c9a. *Source:*
  `assets/live.html:164-175, 327, 343-347`. *Today:* broken — Markdown prose and link text set no
  `overflow-wrap`, so one long unbroken URL in an answer scrolls the whole feed sideways.

### Markdown

- **SE-C90** Transcript text is never markup. Markdown renders to elements with the text as text,
  never through `innerHTML`, so nothing a transcript contains — in prose, a table cell, link text, a
  link's address or a question's option — becomes markup or an attribute. *Why:* the content is
  other people's tool output and web pages, on a page that answers permission prompts and sends
  messages; an unescaped quote let a Markdown link gain an `onmouseover`. *Regression:* yes —
  29c5eb3; check `markdown survives hostile input`. *Source:*
  `assets/live.html:475-478, 747-748, 751`, `scripts/check-live-ui.js:292-296`.
- **SE-C91** Only an `http:` or `https:` address becomes a link (`[text](https://…)`), opening in a
  new tab with no opener; any other scheme — `javascript:`, `data:`, a relative path — stays as
  typed text. *Why:* a link is the one place a transcript chooses where the reader goes. The plan's
  "https only" is narrower than the web, whose semantics are the rule (Decision F). *Regression:*
  no. *Source:* `assets/live.html:756`. *Phones:* Android `bold italic code and links become runs`.
- **SE-C92** A bare URL is text, not a link. *Why:* the web's semantics (Decision F); Android's
  inline parser links bare URLs today and is brought to the web. *Regression:* no. *Source:*
  `assets/live.html:756`. *Phones:* Android `bold italic code and links become runs`.
- **SE-C93** A fenced block, from three backticks to three backticks, is code and verbatim: no
  emphasis, inline code or link is read inside it, its language tag is dropped, a newline before the
  closing fence is dropped, and it scrolls sideways rather than wrapping. *Why:* code read as
  Markdown is code changed. *Regression:* no. *Source:*
  `assets/live.html:328-330, 751-752, 793`. *Phones:* iOS `testCodeFencesSplitOutOfProse`.
- **SE-C94** An unterminated fence is not code: its backticks and everything after them stay
  prose. *Why:* the web's semantics (Decision F). *Regression:* no. *Source:*
  `assets/live.html:751`. *Phones:* iOS `testCodeFencesSplitOutOfProse` expects the opposite
  (`.code("unterminated")`) and is brought to the web.
- **SE-C95** Inline code, from one backtick to the next on the same line, is code and
  verbatim. *Why:* code read as Markdown is code changed. *Regression:* no. *Source:*
  `assets/live.html:753-756`. *Phones:* Android
  `code inside emphasis and emphasis inside links keep their styles`. *Today:* broken — emphasis and
  links are applied after inline code is marked, so `**x**` inside backticks renders bold within the
  code, and a Markdown link inside backticks becomes a link.
- **SE-C96** `**bold**`, on one line with no asterisk inside, is bold. `*italic*` is italic only
  when its opening asterisk follows the start of the text, whitespace or "(", so `d*no*e`, `*.rs`,
  `2 ** 8` and a lone `*` stay as typed; underscores never mean emphasis. *Why:* globs, arithmetic
  and snake_case are common in transcripts and are not emphasis. *Regression:* no. *Source:*
  `assets/live.html:754-755`. *Phones:* Android `what is not markdown is left as typed`.
- **SE-C97** A `* ` list marker is never read as emphasis. *Why:* a bullet that loses its marker
  garbles the list. *Regression:* no. *Source:* `assets/live.html:755, 770`. *Today:* broken — the
  bullet `* item *with* stars` loses its marker, which is read with the next asterisk as emphasis
  around the word "item", leaving `with* stars` behind.
- **SE-C98** A heading is one to six `#`, a space and its text. `#` renders at the third heading
  level, `##` the fourth, `###` the fifth and anything deeper the sixth, so a heading inside a
  message sits under the screen's own; seven `#`, or `#` without a space, is prose. *Why:* the web's
  semantics (Decision F). *Regression:* no. *Source:*
  `assets/live.html:335, 339, 772, 787`. *Phones:* iOS `testMarkdownBlocksMatchTheWebView` and
  Android `markdown blocks match the web view` keep the source level (`heading(1, …)`) and are
  brought to the web.
- **SE-C99** A list item is a line starting with `-` or `*` and a space, or with a number, `.` or
  `)`, and a space; a bullet is read first, so `- 1. x` is a bullet whose words are `1. x`.
  Consecutive items of one kind form one list, and where bullets give way to numbers, or numbers to
  bullets, another list starts, so `- one`, `2. two`, `* three` is a bulleted list, a numbered list
  of one and another bulleted list. A blank line or any other line ends a list, and indentation is
  ignored, so nothing nests. A numbered list is drawn as an ordered list, numbered 1., 2., 3. from
  its first item, never with the source's own numbers, so a list after a blank line counts from 1
  again. *Why:* decided on 2026-10-05 in the phones' favour (Decision F): the page drew every list
  bulleted and dropped the numbers, and a numbered list says an order that bullets lose; the phones
  kept a numbered list apart, and numbered it from 1. *Regression:* no. *Source:*
  `assets/live.html:333-334, 770-771, 785-786, 791` (the page's one list);
  `android/app/src/main/java/ca/whnet/agentrouter/data/Markdown.kt` and
  `ios/AgentRouter/Sources/Model/Markdown.swift` (`flushList`); `fixtures/live/markdown-blocks.json`.
  *Phones:* iOS `testMarkdownBlocksMatchTheWebView`, `testMarkdownBlocksMatchTheSharedFixtures`;
  Android `markdown blocks match the web view`, `SharedFixtureTest`.
- **SE-C100** A quote is a line starting with `>`, the space after it optional, and each such line
  is a quote block of its own. *Why:* the web's semantics (Decision F). *Regression:* no. *Source:*
  `assets/live.html:341-342, 773, 788`. *Phones:* iOS `testMarkdownBlocksMatchTheWebView`; Android
  `markdown blocks match the web view` (each pins a single quoted line; the plan records that the
  phones treat runs of quoted lines differently, and they are brought to the web).
- **SE-C101** A table is a row of pipes immediately followed by a separator row of pipes, dashes,
  colons and spaces; the pipe rows after it are its body, up to the first other line. Without the
  separator, a line of pipes is prose. Cells are trimmed and keep their inline formatting, and
  alignment colons are accepted and ignored. *Why:* a sentence containing pipes must stay a
  sentence, while plans and status grids, the content most worth reading from a phone, arrive as
  tables. *Regression:* no. *Source:* `assets/live.html:759-766, 774-784`. *Phones:* iOS
  `testMarkdownBlocksMatchTheWebView`; Android `markdown blocks match the web view`
  (`| not | a table |` stays prose).
- **SE-C102** A table scrolls sideways inside the column rather than widening it, its cells
  unwrapped. *Why:* the same correction terminal blocks needed: a wide table would push the page
  sideways. *Regression:* no. *Source:* `assets/live.html:343-348`.
- **SE-C103** Every other non-blank line is a paragraph of its own: consecutive lines are not
  joined, and blank lines add nothing. *Why:* the web's semantics (Decision F). *Regression:*
  no. *Source:* `assets/live.html:336-338, 789`. *Phones:* iOS `testMarkdownBlocksMatchTheWebView`
  and Android `markdown blocks match the web view` join lines (`paragraph("plain\nlines")`) and are
  brought to the web.
- **SE-C104** Markdown never drops text: every character of the source appears, except the markers
  it interprets. *Why:* a renderer that loses words misreports the transcript. *Regression:*
  no. *Source:* `assets/live.html:751`. *Today:* broken — a fence opened and closed on one line
  (three backticks, `x`, three backticks) takes `x` for its language and draws an empty block, and a
  fence tagged `c++` keeps `++` as its first line of code.

### Question form

- **SE-C105** A question row is a card in the question colour, set apart from the flow, holding a
  form drawn from the transcript: one tab per question, named by its header or else "Question N",
  the first one open, and a ✓ on each tab once its question has an answer. *Why:* a question is the
  one thing worth interrupting the flow for; the editor puts each question behind its own tab, and
  one long scroll left the later questions out of reach on a phone. *Regression:* no. *Source:*
  `assets/live.html:224-235, 821-836, 854-865, 967-969`.
- **SE-C106** A question takes one choice, or several when it says `multiSelect`; each option shows
  its label and its description, and a chosen option is marked. *Why:* the tool says
  which. *Regression:* no. *Source:* `assets/live.html:238-252, 840-846, 861-865`. *Phones:* iOS
  `testStreamItemsDecode`; Android `stream items decode`.
- **SE-C107** "Other" is always offered, with a field for a typed answer, and typing in that field
  selects Other. *Why:* none of the options may fit, and typed text must not be dropped in silence
  by submitting without picking the choice beside it. *Regression:* no. *Source:*
  `assets/live.html:847-850, 867-872`.
- **SE-C108** Each form's choices are its own: two question forms in one feed never share a
  selection group, their group names coming from `useId()`. *Why:* a pick in one form must not clear
  another's. *Regression:* no. *Source:* `assets/live.html:843, 848`; plan, "Where each part
  goes". *Today:* broken — every form names its groups `q0`, `q1`…, so the first question of every
  form in the feed is one radio group, and a pick in one clears the others.
- **SE-C109** A question's answer is its chosen labels in the tool's option order, then the typed
  Other text, trimmed, all joined by ", ". Other with nothing typed adds nothing, typed Other alone
  is an answer, and a question with nothing chosen is left out. As a message, the answers are one
  line per answered question: "<header, else question>: <answer>". *Why:* that is what a person
  answering by hand would write. *Regression:* no. *Source:*
  `assets/live.html:884-890, 945-956`. *Phones:* iOS
  `testAnswersComposeTheWayThePersonWouldHaveTyped`; Android
  `answers compose the way the person would have typed`.
- **SE-C110** An option's label round-trips exactly, quotes included: the answer sent is the label
  as the tool wrote it. *Why:* a label holding quotes once broke its input so badly that none was
  created. *Regression:* yes — 29c5eb3. *Source:* `assets/live.html:475-478, 843-845`.
- **SE-C111** Submit sends every answered question together; with none answered it says "Pick at
  least one answer first." and sends nothing. The form says the composer is the other way to answer
  ("or type a reply below"). *Why:* one Submit answers them together, as in the
  editor. *Regression:* no. *Source:* `assets/live.html:876-894`.
- **SE-C112** Before sending, the form asks the router what is waiting now. If a permission request
  is waiting, nothing is sent — neither the answers nor a message — and the form keeps its choices
  and says "Not sent: a permission prompt is waiting first — answer it, then this." beside its
  Submit. *Why:* unnamed, the router answers whatever single prompt is waiting, and a permission
  prompt handed these answers would take them as its approval. *Regression:* yes — 3542332 (a stale
  form could approve a Bash command); check `an answer names the prompt it answers`. *Source:*
  `assets/live.html:898-907`, `scripts/check-live-ui.js:799-812`. *Phones:* iOS
  `testAQuestionIsNeverAnsweredOntoAPermissionRequest` (in its own words: "A permission request is
  waiting too — answer it first, then submit these again."); Android
  `a question's answers never land on a permission prompt` (Android names a question it saw waiting
  under a newer permission request rather than refusing, a known difference).
- **SE-C113** If a different question is waiting — not the same questions in the same order —
  nothing is sent, not even as a message, and the form says "Not sent: a different question is
  waiting now." A prompt that does not list its questions, or lists them in a shape that is not a
  list, is taken at its word. *Why:* a form stays live after its question was answered in the
  editor, and its answers named onto the next question would be written into it and dismiss it
  unread. *Regression:* yes — 3542332; check `an answer names the prompt it answers`. *Source:*
  `assets/live.html:908-911, 936-943`, `scripts/check-live-ui.js:799-812`. *Phones:* iOS
  `testAnObsoleteQuestionFormIsNotSentOntoTheLiveOne`; Android
  `a form's answers go only to the question it was drawn for`,
  `a prompt's input is read only for its questions, and never fails the prompt`,
  `a form for a question no longer asked is not sent to the one asked now` (AnswerTest.kt).
- **SE-C114** When the waiting prompt is this form's question, the answer names its `request_id`;
  with nothing to name it goes unnamed, byte for byte the body older clients send
  (`{session, answers}`). *Why:* a named answer lands on exactly that prompt or is refused as gone,
  never on whichever other one waits by then. *Regression:* no. *Source:*
  `assets/live.html:912-916, 1274-1277`, `scripts/check-live-ui.js:784-797, 814-820`. *Phones:*
  Android `an answer names its prompt only when it knows it`,
  `a form for the question asked now answers it by name` (AnswerTest.kt).
- **SE-C115** The form is spent — greyed, its inputs and Submit disabled, saying "answered" — only
  once the router has answered the prompt itself. *Why:* a form that greyed and said "delivered"
  while the prompt stayed open and the session stayed blocked looked as though it had
  worked. *Regression:* yes — 2b421d4. *Source:* `assets/live.html:250, 918-923`.
- **SE-C116** When the router cannot answer directly — an unwrapped session, a prompt already gone —
  the form says why (the router's words at any status, SE-C183, else "could not answer directly")
  and sends the composed answers as an ordinary message, whose notice says it may not
  dismiss the prompt. *Why:* the text is still worth sending, and saying so beats losing what was
  typed. *Regression:* no. *Source:* `assets/live.html:925-928`,
  `scripts/check-live-ui.js:814-821, 1074-1083`.
- **SE-C117** A submit that reaches nothing says "could not answer — <error>" beside the form, and
  the form stays live. *Why:* nothing was answered, so nothing is spent. *Regression:* no. *Source:*
  `assets/live.html:929-931`.
- **SE-C118** Submit is guarded against a double tap: while an answer is in flight it is disabled,
  and the form says "answering…". *Why:* a second tap sends the answers again. *Regression:*
  no. *Source:* `assets/live.html:895-896`; plan, "Where each part goes". *Today:* broken — only the
  hint changes, and a second tap sends again.
- **SE-C119** A form answers the session it was drawn in, even when another session is open by the
  time the router replies; nothing it sends or says lands in another session. *Why:* answers sent to
  the wrong session answer, or approve, the wrong thing. *Regression:* no. *Source:*
  `assets/live.html:903-916, 928`. *Today:* broken — the answer's session is read only after the
  check of what is waiting returns, so switching sessions mid-submit sends the answers to the newly
  open one, and the message fallback goes there too.

### Permission prompts

- **SE-C120** A permission request is a control frame between the CLI and its editor, never a
  transcript row, so the screen asks for it: every two seconds while a session's feed is on screen,
  and at once when a status names a waiting request it has not asked about yet. A failed ask changes
  nothing and is tried again. A question is left to the feed's form. *Why:* a Bash approval once
  buzzed the phone and then offered nothing to tap. *Regression:* yes — 444b0c5. *Source:*
  `assets/live.html:567-569, 1266-1279, 1350-1369`, `scripts/check-live-ui.js:752-760`.
- **SE-C121** The card says what is waiting — "<name> is waiting for permission", the name being the
  prompt's display name, else its tool, else "A tool" — then "N more waiting after this one" when
  the router counts more, and nothing of others when it sends no count; then the prompt's
  description; then what it would act on: the command, file path, path, URL or pattern, else its
  whole input, cut at 400 characters with "…". *Why:* prompts are answered one at a time, newest
  first, and the next is drawn once this one is gone. *Regression:* no. *Source:*
  `assets/live.html:1281-1310`, `scripts/check-live-ui.js:760-763, 866-871`. *Phones:* Android
  `the prompt's count is optional`.
- **SE-C122** The card sits above the composer and wraps its command rather than widening the
  screen. *Why:* it is the thing to deal with first: nothing typed below will run until it is
  answered. *Regression:* no. *Source:* `assets/live.html:261-273, 396`.
- **SE-C123** Every choice is a visible button: Allow, each of the prompt's own suggestions by its
  label, then Deny — and no suggestion this prompt did not offer. *Why:* the router refuses anything
  else, and the editor on the other end would drop it too. *Regression:* no. *Source:*
  `assets/live.html:1311-1327`.
- **SE-C124** Each answer names its prompt's `request_id`: Allow is `{allow: true}`, a suggestion
  `{allow: true, suggestion: <its index>}`, and Deny
  `{allow: false, message: "Declined from the live view."}`. *Why:* with more than one waiting,
  "whatever is waiting" is not something the router will guess at. *Regression:* no. *Source:*
  `assets/live.html:1313-1326`; check `an answer names the prompt it answers`
  (`scripts/check-live-ui.js:764-772`). *Phones:* Android
  `an answer names its prompt only when it knows it`.
- **SE-C125** While an answer is in flight every button on the card is disabled. On success the card
  goes, and a success notice says what was done — "Allowed.", the suggestion's label, "Declined." —
  for five seconds. A refusal says the router's words at any status (SE-C183), else "Could not
  answer that prompt.", and gives the buttons back. *Why:* one tap, one answer, and a refusal in
  the router's own words. *Regression:* no. *Source:* `assets/live.html:1330-1347`.
- **SE-C126** A 409 means the prompt is no longer waiting — answered elsewhere, or withdrawn — so
  the card, which can only fail again, is replaced at once by whatever is waiting now, if anything
  is. *Why:* a dead card left up invites a tap that cannot work. *Regression:* no. *Source:*
  `assets/live.html:1340-1342`, `scripts/check-live-ui.js:774-782`.
- **SE-C127** The card is redrawn when the prompt changes and when the count behind it changes, and
  not otherwise. *Why:* redrawn on every poll, the card would be reset under the reader every two
  seconds. *Regression:* no. *Source:* `assets/live.html:1365-1367`.
- **SE-C128** A prompt this screen has answered is not drawn again while the router catches
  up. *Why:* the wrapper clears a prompt only once it sees the answer go past, so the next poll may
  still list it, and a second tap on it is refused. *Regression:* no. *Source:*
  `assets/live.html:1338, 1366-1367`,
  `android/app/src/main/java/ca/whnet/agentrouter/feed/LiveFeed.kt:101, 410-428`. *Today:* broken —
  the next two-second poll can draw again the card just answered.
- **SE-C129** A card belongs to the session it was drawn for: opening another session clears it at
  once, a reply that comes back after another session opened is dropped, and with no session's feed
  on screen — the list, or Find — there is no card. *Why:* a card answered in the wrong session
  approves the wrong thing. *Regression:* no. *Source:* `assets/live.html:1161-1163, 1350-1364`.
- **SE-C130** The card never takes focus and never answers on Enter; only a deliberate press on one
  of its buttons answers it. *Why:* an Enter meant for the composer could approve a prompt. Carbon's
  `ActionableNotification` is an alertdialog that takes focus, so the card is a
  `Callout`. *Regression:* no. *Source:* `assets/live.html:261-262, 1311-1319`; plan, "Carbon traps
  already known".

### Facts strip

- **SE-C131** An open session has a facts strip: its model without the "claude-" prefix ("unknown"
  when the transcript names none), its effort when known, its total tokens with cache reads
  included, its output and cached tokens, its turns ("1 turn", "2 turns") and its cost when it is
  priced. *Why:* what a session is and what it is costing, at a glance; a total without cache reads
  would say a long session cost nothing. *Regression:* no. *Source:*
  `assets/live.html:303-311, 1192-1199`.
- **SE-C132** The project's own total — sessions, tokens, turns and cost — sits apart on the same
  strip, and only when the project has more than one session. *Why:* it answers a different
  question: what is this costing me overall. *Regression:* no. *Source:*
  `assets/live.html:308-309, 1187-1190, 1200-1204`.
- **SE-C133** Totals are read once each time a session opens, never pushed with every event. *Why:*
  they need a scan of the whole file, and the numbers do not move fast enough to be worth
  streaming. *Regression:* no. *Source:* `assets/live.html:1180-1181`.
- **SE-C134** Money reads "$0.123" under a dollar, "$4.56" under a thousand and "$1,234" above;
  tokens read "1.2B", "3.4M", "56k" or the plain number; a cost the router could not price is left
  out, never shown as $0. *Why:* cents matter at coffee prices and are noise at four
  figures. *Regression:* no. *Source:* `assets/live.html:467-474, 1199, 1203`.
- **SE-C135** While the totals are being read the strip says "reading totals…"; a read that fails,
  times out or answers a refusal says so or leaves the strip out — never "unknown", zeros or
  "undefined turns" read off a refusal. *Why:* missing is not zero (Northstar `docs/13`
  §1). *Regression:* no. *Source:* `assets/live.html:1183-1210`, `src/live.rs:1345-1348`. *Today:*
  broken — a 404 `{ok: false, reason: "No such session."}` is read as totals and prints "unknown · 0
  tokens · 0 out · 0 cached · undefined turns", and a read that never answers leaves "reading
  totals…" for good.
- **SE-C136** The strip and its switchers are drawn only for the session they were read for, and a
  switcher only ever changes that session. *Why:* a slow read for one session must not dress
  another, nor aim its controls at it. *Regression:* no. *Source:*
  `assets/live.html:1185-1209, 1253-1256`. *Today:* broken — a slow read draws one session's model,
  effort, mode and switchers under the next session opened, and those switchers then change the
  session on screen.

### Switchers

- **SE-C137** A wrapped session's strip carries three switchers — model, effort and mode — and an
  unwrapped session's strip just says what it is. *Why:* only a session under the wrapper can be
  switched. *Regression:* no. *Source:* `assets/live.html:1206-1208`.
- **SE-C138** What a session can be switched to comes from the router (`/api/live/options`), one
  list for every client: models and efforts are bare values, and modes carry the router's own
  labels. *Why:* the wording lives in the router, not in three clients. *Regression:* no. *Source:*
  `assets/live.html:412-414, 1221-1227`. *Phones:* iOS `testSwitchPayloadsDecode`.
- **SE-C139** A switcher leads with the current value, marked as such ("model: opus") and offered
  even when the router's list lacks it; choosing it again does nothing. *Why:* the reader must see
  what is in force before changing it. *Regression:* no. *Source:*
  `assets/live.html:1228-1235, 1244`.
- **SE-C140** A switcher whose current value is unknown says so, and never shows its first option as
  though that were current. *Why:* a choice displayed as current is read as the
  session's. *Regression:* no. *Source:* `assets/live.html:1228-1235`. *Today:* broken — with no
  effort or mode on record, the switcher shows "low" (or the first mode) as the session's.
- **SE-C141** The model switcher ends with "other…", which asks in a modal for a model name or alias
  ("opus, sonnet[1m], a full id…"); an empty answer changes nothing. *Why:* the router's list is not
  every model the CLI accepts. *Regression:* no. *Source:* `assets/live.html:1236-1243`; plan,
  "Where each part goes". *Today:* broken — it uses the browser's `prompt()`, which does nothing in
  VS Code's sandboxed fallback, so "other…" cannot be used there.
- **SE-C142** Choosing to bypass permissions asks first, in a modal — "This session will stop asking
  before it runs commands or edits files" — and declining puts the switcher back. *Why:* one tap
  would otherwise stop the session asking about anything, on a machine the person holding the phone
  may not be sitting at; all three clients confirm first. *Regression:* no. *Source:*
  `assets/live.html:1245-1250`. *Today:* broken — it uses the browser's `confirm()`, which is
  blocked in VS Code's sandboxed fallback, so Bypass can never be chosen there.
- **SE-C143** While a switch is in flight its switcher is disabled. The outcome is the router's: the
  CLI's own notices when it sends any, "<kind> sent — the session has not confirmed yet" while it is
  pending, else "<kind> set to <value> — takes effect on the next turn", as a success for six
  seconds. *Why:* the choice takes effect on the next turn, and the CLI's own notice, or refusal, is
  what is shown. *Regression:* no. *Source:* `assets/live.html:1213-1214, 1251-1258`. *Phones:* iOS
  `testSwitchPayloadsDecode`.
- **SE-C144** A refused switch says the router's words at any status (SE-C183) — the CLI's own
  refusal, such as bypass for a session not launched for it — else "Could not set the <kind>.", and
  puts the switcher back. *Why:* two modes can be refused as they are asked for, and
  only the CLI's sentence says why. *Regression:* no. *Source:* `assets/live.html:1259-1260`,
  `scripts/check-live-ui.js:1057-1066`.
- **SE-C145** After a successful switch, the switcher's current value is the new one, and choosing
  the previous value again sends it. *Why:* a switch back is a switch. *Regression:* no. *Source:*
  `assets/live.html:1228, 1244`. *Today:* broken — the switcher keeps comparing against the value it
  was drawn with, so switching back is a silent no-op and its "model:" mark stays on the old value.

### Composer

- **SE-C146** The composer belongs to an open session: it is not shown on the list or in
  Find. *Why:* a message needs a session to go to. *Regression:* no. *Source:*
  `assets/live.html:395, 1150, 1390, 1435`.
- **SE-C147** Enter sends and Shift+Enter starts a new line; the box grows with its text up to 120
  px and shrinks back after a send. *Why:* it is a chat box. *Regression:* no. *Source:*
  `assets/live.html:283-284, 1554-1559, 1608`; plan ("a `TextArea` that grows to 120 px").
- **SE-C148** Send trims the text, sends nothing when it is empty, and is disabled while a message
  is in flight. *Why:* one press, one message. *Regression:* no. *Source:*
  `assets/live.html:1587-1589, 1628, 1631`.
- **SE-C149** Send stays available while a turn runs, with Stop beside it: a message typed then is
  queued by the session itself and taken up when the turn ends, and the header counts it
  (SE-C62). *Why:* queuing is the CLI's own behaviour. The stylesheet's note that Stop takes Send's
  place is stale. *Regression:* no. *Source:* `assets/live.html:300, 559-561, 565`.
- **SE-C150** A successful send is echoed at once as a dashed, faded bubble labelled "you · queued";
  the box is cleared and the feed follows to the bottom. *Why:* the real row arrives only when the
  session writes it to its transcript, which on a busy session can be minutes, and without the echo
  a successful send looks identical to a broken one. *Regression:* no. *Source:*
  `assets/live.html:185-188, 302, 1596-1608`, `scripts/check-live-ui.js:619-621`.
- **SE-C151** A send that went over the peer socket (`framed`) warns, until dismissed, that the
  session will see it as coming from another Claude rather than from you, and that installing the
  wrapper in Settings and restarting the session makes it arrive as your own message. *Why:* the
  session will often ask who sent it instead of doing the thing, and the reader would otherwise
  think the message was mangled in transit. *Regression:* no. *Source:*
  `assets/live.html:1609-1614`. *Phones:* iOS `testStreamItemsDecode`; Android `stream items decode`
  (`framed`).
- **SE-C152** Answers sent as a message say "Sent as a message — it may not dismiss the prompt."
  until dismissed; an ordinary send says "Delivered as your message." for four seconds. *Why:*
  measured: a queued user turn does not satisfy a pending AskUserQuestion, so "delivered" there
  would be true and misleading. *Regression:* yes — 2b421d4. *Source:* `assets/live.html:1615-1622`.
- **SE-C153** A send that fails keeps the typed text and says why: the router's words at any status
  (SE-C183), else "Send failed (<status>)"; no reply at all says "Send failed — <error>". *Why:*
  nothing typed is lost to a refusal. *Regression:* no. *Source:* `assets/live.html:1623-1628`.
- **SE-C154** Sending a question's answers as a message never touches the composer's own
  draft. *Why:* the draft is the reader's, and unrelated to the form. *Regression:* no. *Source:*
  `assets/live.html:928, 1608`. *Today:* broken — every successful send clears the box, so a draft
  typed there is erased when a form falls back to a message.
- **SE-C155** ArrowUp recalls what the person has already said in this session — every message of
  theirs in the transcript, not a subagent's, a consecutive repeat once, the newest 200 — from the
  moment the session opens. *Why:* the history was empty until something had been sent in that page
  load, so on a freshly opened session ArrowUp did nothing. *Regression:* yes — 9cbe4fc. *Source:*
  `assets/live.html:451-454, 1087-1092, 1154`, `scripts/check-live-ui.js:626`.
- **SE-C156** The arrows walk the history only when the caret is on the edge it would leave —
  ArrowUp at the very start, ArrowDown at the very end, nothing selected — so a multi-line draft is
  navigated normally. ArrowUp from a draft keeps it, ArrowDown past the newest entry brings it back,
  ArrowUp at the oldest stays put, and the caret lands at the end. *Why:* the arrows are also how a
  draft is edited. *Regression:* no. *Source:* `assets/live.html:1560-1585`.
- **SE-C157** Stop shows beside Send only while the stream vouches for a stoppable status (SE-C10),
  never on "connecting…" or "reconnecting…". An unwrapped session keeps it and is told by the router
  on click. *Why:* a stop aimed at a picture that may be minutes old could land on a turn nobody has
  seen. *Regression:* yes — 3542332; check
  `a dropped stream leaves the header stale, not showing its last word`. *Source:*
  `assets/live.html:559-562, 572-582`. *Phones:* iOS `testStopIsOfferedOnlyForARunningTurn` (the
  iPhone also hides Stop for an unwrapped session, a known difference).
- **SE-C158** Stop is a labelled button in the warn colour. While it is in flight it is disabled;
  success says "Stopped." for four seconds; a refusal says the router's words at any status (the
  turn is over, only background work runs; SE-C183), else "Could not stop this session.". *Why:*
  Stop is the one action that interrupts work. *Regression:* no. *Source:*
  `assets/live.html:301, 401, 1371-1385`,
  `scripts/check-live-ui.js:1084-1089`. *Today:* broken — the button's face is a bare "■", its words
  only in a hover title.
- **SE-C159** The side question ("btw") asks something of the session's context in a fork the
  session never sees — what `/btw` does, which the editor's stream refuses — and is shown on this
  screen only. Its question is the composer's text, else asked for in a modal ("answered from this
  session's context, without interrupting it"); with nothing typed, nothing is asked. *Why:* a quick
  question must not interrupt the session. *Regression:* no. *Source:*
  `assets/live.html:1633-1642`. *Today:* broken — with an empty box it uses the browser's
  `prompt()`, which does nothing in VS Code's sandboxed fallback.
- **SE-C160** A side question is echoed as a pending "you · side question" bubble and the box is
  cleared, and a notice says "asking on the side — this replays the session once, usually under a
  minute" until the answer comes. The answer lands as a "side answer" row, in Markdown, "(no
  answer)" when empty, with "answered in <time> · <cost>" for six seconds; a failure says the
  router's words at any status (SE-C183), else "The side question failed."; the button is disabled
  meanwhile. *Why:* it takes the better part of a minute, and the reader must see that it
  is under way. *Regression:* no. *Source:* `assets/live.html:1643-1659`. *Phones:* iOS
  `testStreamItemsDecode` (the side answer's reply).

### Find

- **SE-C161** Find searches the router's whole index of what it has recorded, and from a session it
  is the only way to reach that session's earlier turns. *Why:* the feed only ever shows the
  transcript's tail: the stream backfills 512 KB and there is no scrolling further up. *Regression:*
  no. *Source:* `assets/live.html:80, 1408-1411`.
- **SE-C162** Opened from a session, Find is scoped to "This session"; opened from the list, to
  "Everywhere", with "This session" disabled, there being nothing to narrow to. The scope is a
  two-way switch, always visible, and changing it searches again at once. *Why:* scoping to the open
  session is the point of having Find here rather than on the dashboard. *Regression:* no. *Source:*
  `assets/live.html:387-390, 1429-1434, 1463-1468, 1474-1480`; check
  `find opens from the list and from a session, and closes back to where it was`.
- **SE-C163** The field says what it searches — "Find anywhere in this session" or "Find in every
  session" — and stays in view with the scope while the hits scroll. Opening Find focuses the field,
  selects its text and runs the query it holds. *Why:* the reader types straight away. *Regression:*
  no. *Source:* `assets/live.html:83-92, 1440-1441, 1469-1470`.
- **SE-C164** Typing searches 200 ms after the last keystroke, and a reply to an older query never
  replaces a newer one's; an empty query shows nothing and asks nothing. *Why:* a stale answer could
  otherwise overwrite a newer one. *Regression:* no. *Source:* `assets/live.html:1481, 1484-1497`.
- **SE-C165** A search asks for 30 hits, best match first, and says so when the router counts
  more. *Why:* truncation is loud (Northstar `docs/13` §1), and the router's `sessions` roll-up
  counts every match. *Regression:* no. *Source:* `assets/live.html:1487`,
  `src/api.rs:657-670`. *Today:* broken — the cap is silent.
- **SE-C166** No match says so — "Nothing matched <query> in this session." or "Nothing matched
  <query>." — never an empty pane. *Why:* an empty answer must not look like no
  answer. *Regression:* no. *Source:* `assets/live.html:1500-1506`; check
  `an empty result says so rather than showing nothing`.
- **SE-C167** A search that did not reach the router says "Could not reach the router.", and one the
  router failed says its error; neither ever reads "Nothing matched". *Why:* a failed search is not
  an empty one. *Regression:* no. *Source:* `assets/live.html:1490-1497, 1501`,
  `src/api.rs:122-130`. *Today:* broken — a 500 `{error}` is read as zero hits and shows "Nothing
  matched".
- **SE-C168** When the router ran something other than what was typed — its retry as plain words,
  for a query its search language rejected — Find says "Searched for <expression>", and otherwise
  says nothing of it. *Why:* it is only worth saying when it differs. *Regression:* no. *Source:*
  `assets/live.html:1508-1511`, `src/api.rs:647-663`.
- **SE-C169** Each hit shows its kind in the reader's words — message, tool call, tool result,
  thinking, system prompt, else the kind as sent — its role, how long ago it was, and its snippet in
  at most five lines, wrapping anywhere. A hit from another session also names its project ("—" when
  unknown) and its session's first eight characters. *Why:* where a match is matters as much as what
  it says. *Regression:* no. *Source:* `assets/live.html:97-100, 1426-1427, 1515-1528`; check
  `a hit renders, and one already in the open session is not a link to it`. *Phones:* Android
  `the search reply decodes`, `a hit whose project is null still decodes` (SearchTest.kt).
- **SE-C170** Matches are marked by splitting the snippet on the ‹ › pair, never by substituting
  into it: a marker the conversation itself contained can only shift which run is emphasised, and
  never leaves a mark open, drops text or lets the snippet become markup. *Why:* substitution could
  leave a run open and take the rest of the snippet with it. *Regression:* no. *Source:*
  `assets/live.html:1421-1424`; check
  `a stray marker cannot leave a tag open, and snippet text is escaped`. *Phones:* Android
  `a snippet splits into plain and matched runs`, `a stray marker cannot swallow the rest`
  (SearchTest.kt).
- **SE-C171** A hit is identified by its session, block and ordinal together, never by its block
  alone. *Why:* a block appears at every place it was replayed, and a list given a duplicate key
  crashes rather than coping. *Regression:* no. *Source:* `assets/live.html:1512-1529`. *Phones:*
  Android `the search reply decodes` (SearchTest.kt).
- **SE-C172** A hit in the session already open is not a link, since the feed cannot scroll to a
  turn it never loaded; a hit elsewhere opens its session. *Why:* a link that lands nowhere is worse
  than none. *Regression:* no. *Source:* `assets/live.html:1513-1518, 1533-1543`; check
  `a hit renders, and one already in the open session is not a link to it`.
- **SE-C173** Opening a hit looks its session up in the router's list. A session no longer listed
  says "That session is no longer listed." by the hit, a hosted router says why it cannot open it —
  the session is not on a machine connected to it — and a list that could not be read says that;
  none of them ever reads "no longer listed" for another reason. *Why:* "no longer" is the wrong
  reason when the router lists nothing or could not be asked. A hosted router gives its reason as
  "That session is not on a connected machine" (`OFF_MACHINE`). *Regression:* no. *Source:*
  `assets/live.html:466, 1533-1543`, `scripts/check-live-ui.js:1113-1115`. *Today:* broken — a list
  read that fails is taken for an empty list, so it reads "no longer listed", and the notice goes to
  the composer, which Find hides.
- **SE-C174** Find never touches the stream: the open session stays connected and keeps receiving
  while the hits are on screen, and closing Find returns to it, its strip redrawn and at its newest
  row, or to the list. *Why:* coming back from a search must not lose the live view. *Regression:*
  no. *Source:* `assets/live.html:1413-1414, 1444-1461`; check
  `find opens from the list and from a session, and closes back to where it was`.
- **SE-C175** Find is part of navigation: its query and scope live in the URL, Back closes it, and a
  refresh keeps it. *Why:* a pane that Back does not close is left open beside whatever Back
  shows. *Regression:* no. *Source:* `assets/live.html:1406, 1473, 1545-1552`; plan, "One app in
  Northstar's shell". *Today:* broken — Find is not in the URL, and both the ‹ button and the
  browser's Back go to the list or a session without closing Find, leaving its pane open beside
  them.
- **SE-C176** Escape closes Find at once, wherever the focus is, and the Find button toggles
  it. *Why:* Find is a pane laid over the screen, and Carbon's `Search` swallows Escape to clear its
  box, so the close is handled on the field itself. *Regression:* no. *Source:*
  `assets/live.html:1473, 1482`; plan, "Carbon traps already known".

### Notices

- **SE-C177** A notice is text, never markup. *Why:* some notices carry a reason the router
  supplied. *Regression:* no. *Source:* `assets/live.html:428-429, 436-437`.
- **SE-C178** A notice appears next to what it concerns — a send's or a side question's by the
  composer, a switch's by the strip, an answer's by its card or form, an archive's by its row or
  tab, a hit's by the hit — framed, dismissible, with success and warning told apart, on a timer of
  the screen's own. *Why:* a warning about how a message was delivered belongs where it is read, not
  under the thing it is about. *Regression:* no. *Source:*
  `assets/live.html:286-299, 395-397, 430-445`; plan, "Where each part goes". *Today:* broken — one
  notice above the composer serves every outcome.
- **SE-C179** A notice is never written into something hidden. *Why:* an outcome nobody can see is
  an outcome lost. *Regression:* no. *Source:* `assets/live.html:395, 1038, 1047, 1540`. *Today:*
  broken — a failed Restore from the list and "That session is no longer listed." from Find go to
  the composer's notice, which both of those views hide.
- **SE-C180** A success notice clears itself after four to six seconds; a warning or a failure stays
  until it is dismissed or replaced; a new notice replaces the one before and restarts the
  timer. *Why:* a success needs no answer, and a failure does. *Regression:* no. *Source:*
  `assets/live.html:430-445, 1258, 1338, 1380, 1621, 1655`.
- **SE-C181** Showing a notice, clearing it and showing another, with or without a timer and in any
  order, never throws. *Why:* clearing a notice once called itself, blew the stack inside
  `openSession` and left every session blank. *Regression:* yes — 3fe8c5d; check
  `notices do not recurse`. *Source:* `assets/live.html:431-445`,
  `scripts/check-live-ui.js:284-290`.
- **SE-C182** Opening a session or going back to the list clears the notice. *Why:* a notice belongs
  to the view it was raised in. *Regression:* no. *Source:* `assets/live.html:1155, 1394`.
- **SE-C183** A refusal is said in the router's own words whenever it sends any, at any status, as
  both phones say them: a hosted router's 503 for a machine that is offline, its 502 when the relay
  to the machine failed, a local router's own 500. The router's words are a JSON body's `error`,
  `reason` or `message`, or the one `agent-router: …` line its guard answers with — never a page
  from something in front of the router, such as an ingress's HTML while the router is down. A
  sentence naming the action that failed is only for a refusal that carried no words. *Why:*
  whichever action the reader tried, what the router said is what they are told, not a generic
  failure, and what something in front of it said is not the router's; decided on 2026-10-05,
  when a hosted router's 502 on Send read "Send failed (502)" where both phones showed the relay's
  sentence. *Regression:* no. *Source:*
  `assets/live.html:422-425, 927, 1038, 1259, 1339, 1381, 1624, 1657`; check
  `a per-session action a hosted router refuses says what it said`
  (`scripts/check-live-ui.js:1050-1091`).
- **SE-C184** A reply that is a 2xx carrying `ok: false` is a failure for every action, as the
  permission answer already treats it. *Why:* one fetch layer reads refusals one way; the router
  answers several refusals elsewhere as 200 with `ok: false` (plan, "Data"). *Regression:*
  no. *Source:* `assets/live.html:918, 1037, 1258, 1338, 1380, 1596, 1653`; plan, "Data".
- **SE-C185** Every request has a timeout suited to what it asks — a side question gets the three
  minutes the router itself allows it — and a control disabled while its request is in flight comes
  back when it times out, saying so. *Why:* a router that never answers must not leave a button
  dead. *Regression:* no. *Source:*
  `assets/live.html:1251, 1261, 1331, 1373, 1384, 1589, 1628, 1643, 1659`, `src/live.rs:2179`; plan,
  "Data". *Today:* broken — no request has a timeout, so a router that never answers leaves Send,
  Stop, btw, a switcher or a card disabled for good.

### Hosted router

- **SE-C186** The screen learns what the router is from `/api/auth`'s `mode`: `server` is a hosted
  router, and a router that predates the field, or does not answer, is local, so the screen keeps
  working against it. *Why:* a hosted router has no Mac to assume. *Regression:* no. *Source:*
  `assets/live.html:415-419`; check
  `a hosted router says why its list is empty, and warns of no wrapper`. *Under decision J:* the console ships inside the router it reads, so the older-router case here never reaches it; the rest of the rule stands, and the phones still meet older routers.
- **SE-C187** A hosted router's empty list says why it is empty and where to connect a machine,
  and never blames the week as a local router's empty list does. Under "No sessions here yet" it
  says "No machine has connected yet.", and links to the Machines screen, "Connect a machine",
  where the installer's commands are (MA-C2); it never repeats the commands. *Why:* live sessions
  run on machines, which a hosted router reaches only through their agent, so blaming the week
  would be false. The commands were first given here, decided on 2026-10-07 when the sentence named
  `agent-router agent install --server <this router's address>` with its address left a
  placeholder; the same day they moved to the Machines screen, so they have one home, because a
  list that holds one machine's sessions is never empty and its readers never saw them.
  *Regression:* no. *Source:* `assets/live.html:415-421, 647-648`,
  `scripts/check-live-ui.js:1106-1111`; `ui/src/screens/Sessions/List.tsx`.
- **SE-C188** A hosted router never warns that an editor is not wrapped, and the wrapper's state is
  not even asked for. *Why:* it wraps nothing and has no editor to warn about. *Regression:*
  no. *Source:* `assets/live.html:624-628`, `scripts/check-live-ui.js:1112`.

A hosted router hears its sessions from an agent on each machine (commit `60cf5f8`). Every row of
`/api/live/sessions` names its machine, `machine: {id, label}`, and the router relays each
session's requests to the machine holding it. `GET /api/machines` lists the machines (label,
platform, build, online, since); neither phone reads it, and the console's Machines screen does
(MA-C3).

- **SE-C219** On a hosted router whose rows name more than one machine, the list is headed per
  machine: each machine's label heads a part of its own, and under it is what that machine's
  sessions alone make — its own projects (SE-C23, SE-C24), its own older group (SE-C29 to SE-C31)
  and its own shelf (SE-C32, SE-C33). One project on two machines is two projects, one under each
  heading. A row never names its machine, since its heading does. With one machine, or no labels,
  nothing is named and there is no machine heading: the list is exactly a local router's. *Why:*
  decided on 2026-10-05 in the phones' favour. The page keyed each project by its machine
  ("agent-router · Laptop") in one alphabetical list, with one older group and one shelf, and named
  the machine on every card ("· on Laptop"); the phones head each machine, and a machine's older
  sessions and shelf belong under it. On the console the parts are also tabbed (SE-C226, decided on
  2026-10-07): its All tab is this list, headed per machine, and each machine's tab shows its part
  alone, without the heading its tab takes the place of; the phones draw the parts one after
  another. The parts themselves, and what each holds, are the three clients' contract and the
  fixtures' as before. *Regression:* no. *Source:* `src/machines.rs:365-410`
  (`sessions`, which labels every row); `assets/live.html:658-687` at `60cf5f8`;
  `android/app/src/main/java/ca/whnet/agentrouter/data/SessionGroups.kt:61-126` and
  `ios/AgentRouter/Sources/Model/SessionGroups.swift:129-166` (`byMachine`);
  `fixtures/live/session-groups.json`. *Phones:* iOS `testTwoMachinesEachGetTheirOwnHeading`,
  `testOneMachineOrNoneListsExactlyAsBefore`; Android `two machines are grouped by machine, then as
  before`, `one machine, or none, is the list exactly as it always was`.
- **SE-C220** Machines are in the alphabetical order of their labels, case aside, so a machine keeps
  its place while its sessions move. A label is read trimmed, and a blank one names no machine. Rows
  that name no machine, beside machines that do, are kept: they come last, under a heading of their
  own, "Unnamed machine". A machine with nothing recent still has its heading, over its older group
  or its shelf. *Why:* the phones' rules, held on all three clients by the fixtures; a row the
  router did not label must not vanish from the list. Three differences between the phones are kept
  out of the fixtures. Two labels that differ only in case stay two machines: the console and the
  iPhone put the capitalised one first, and Android keeps the order the router listed them in.
  Android orders punctuation by character code, where the console and the iPhone follow the
  locale, so `mac_mini` and `mac-mini` swap. The iPhone names a machine whose label is blank by the
  start of its id, where Android and the console name none. *Regression:* no. *Source:* as
  SE-C219. *Phones:* iOS `testTwoMachinesEachGetTheirOwnHeading` (the row that names no machine);
  Android `rows that name no machine come last, and names differing only in case stay two
  machines`, `a row without a machine still decodes, and names none` (a blank label).
- **SE-C221** The open session names the machine it runs on: when a hosted router's row names one,
  the header carries its label beside the project, with the machine icon, read as "on <label>",
  however many machines the list holds, as both phones' session screens do. Its tabs are its
  project's sessions on that machine only: the same project on another machine is another project
  (SE-C44). *Why:* the decision follows the phones on naming the machine wherever a session is
  shown, and which machine a session runs on is what the header cannot otherwise say. *Regression:*
  no. *Source:* `android/app/src/main/java/ca/whnet/agentrouter/ui/SessionScreen.kt:133-142,
  164-186` (its siblings, and the machine's tag);
  `ios/AgentRouter/Sources/Views/SessionView.swift:33-48, 198-206`. *Phones:* no phone test pins the tag; both draw it.
- **SE-C222** A project's name is the last segment of its directory, split on `/` and `\` alike,
  wherever the console names one from a path: a project's heading, the open session's heading, the
  rail, and the palette's sessions, projects and search hits. *Why:* a hosted router hears from
  Windows machines, whose directories are `C:\…`; split on `/` alone, such a session or project is
  named by its whole path. The page's `base` split on both from `60cf5f8`; the console's rail and
  palette split on `/` alone until this rule. *Regression:* no. *Source:* `assets/live.html:480` at
  `60cf5f8`; `ui/src/data/projects.ts`. *Phones:* both name a live session by its directory split
  on both (`LiveSession.projectName`), and `session-groups.json` holds a Windows `cwd`; since
  2026-10-05 their dashboards, usage lists and search hits do too, by one rule each
  (`ProjectPath`), with the console's edge cases: iOS `WindowsProjectNameTests`, Android
  `WindowsProjectNameTest`.
- **SE-C223** A per-session request that a hosted router answers 503, naming the machine — "the
  machine running this session (<label>) is offline" — means that machine is offline: it comes
  back later, and the session has not ended. While the stream is refused so, the header says the
  router's sentence in place of "reconnecting…", the pip neutral and Stop gone; the rows already
  read stay; nothing is drawn closed or ended; the stream is asked for again every ten seconds
  (SE-C67); and the first thing a connection delivers ends the sentence. An action refused so says
  the same sentence (SE-C183). `EventSource` reports no status, so the console learns the refusal
  from the same session's prompt, which the router routes as it routes the stream (SE-C60). *Why:*
  a laptop asleep or off the network is a machine offline, not a session over: read as a dropped
  link the header said only "reconnecting…", and read as an end it would cost the reader a live
  session. *Regression:* yes — both phones read the 503 as a bare status and said nothing, fixed in
  `9c2f212`. *Source:* `src/machines.rs:58-80` (`Relay`), `src/machines.rs:412-433` (`holder`);
  commit `9c2f212`. *Phones:* iOS `testASessionWhoseMachineIsOfflineReadsAsOfflineUntilItIsBack`;
  Android `a session whose machine is offline reads as offline until it is back`.
- **SE-C224** The hosted router's other answers for a session are said in its words and retried in
  the same way: 404, "No such session.", when no connected machine holds the session, and 502 when
  the relay to its machine failed ("mini did not answer in time"). Only a refusal of the caller
  (401) ends the retrying (SE-C60), and only the router's own `error` event says that a session
  will not stream (SE-C59). *Why:* none of them is the session ending, and a machine that
  reconnects lists its sessions again. *Regression:* no. *Source:* `src/machines.rs:58-80`.
  *Phones:* iOS `testAnErrorStatusOnALiveStreamIsSaidInTheRoutersWords`; Android
  `an error status on a live stream is said in the router's words`.
- **SE-C225** A machine's heading spans the list at every width, above the project headings it
  holds, its label on one line and cut short with an ellipsis rather than widening the list, so
  nothing scrolls sideways at 360 px. A list with one machine or none draws no machine heading.
  The open session's machine gives way before its project does (SE-C202). *Why:* a label is
  whatever the machine's agent was given, its host name by default, and can run long.
  *Regression:* no. *Source:* `src/agent.rs:426` (`--label`, else the host name).

The owner decided on 2026-10-07 that the console tabs the machines (SE-C226 to SE-C230): with
several machines, the parts of SE-C219 one after another were a long scroll, and a prompt on the
last machine was the furthest from the top. The tabs are the console's; the phones keep the parts
one after another, and what each part holds does not change.

- **SE-C226** On a hosted router whose list holds more than one machine, a row of tabs sits above
  the list: All first, then one tab per machine in the order the machines are headed (SE-C220), and
  "Unnamed machine" last when there are rows that name none. All shows the list headed per machine,
  exactly as SE-C219 draws it, so nothing is lost; a machine's tab shows only that machine's part —
  its projects, its own older group and its own shelf — without its heading, since the tab names
  it. With one machine, or on a local router, there are no tabs and the list is as it always was.
  The header's pip stays the whole list's (SE-C35). A machine's older group is the same group on All
  and on its tab: opened on one, it is open on the other, and it keeps its state across every tab,
  refresh and return to the list (SE-C31). *Why:* decided on 2026-10-07: several machines' parts
  one after another made a long scroll, and a machine at a time is what is looked at. *Regression:*
  no. *Source:* `ui/src/screens/Sessions/List.tsx`; `ui/src/model/session.ts` (`machineTabs`).
- **SE-C227** Each tab says at a glance what is at work there and what waits on you, in the
  header's words — its label, then "2 working · 1 waiting", or nothing beside the label when
  neither — and All says it of the whole list. A tab where any session waits on you carries the
  attention dot, so a prompt on a machine that is not shown is never missed. Each tab's accessible
  name is its label and its counts, or "idle" ("Laptop, 1 working · 1 waiting", "win-box, idle"), so
  it says whether something waits; the dot, whose words the counts already say, is not heard twice
  (SE-C20). The panel is labelled by its tab. The tabs are Carbon's, reached by keyboard: the
  arrows, Home and End move between them without choosing one, and Enter or Space chooses, so
  passing over a tab neither draws its machine nor adds a history entry. *Why:* a waiting session
  is the thing to go and look at (SE-C27), and once the machines are tabbed it can be on a tab that
  is not on screen. *Regression:* no. *Source:* `ui/src/screens/Sessions/List.tsx`
  (`MachineTabLabel`); `ui/src/model/session.ts` (`machineTabs`, which counts as `listHeader`
  does).
- **SE-C228** The tab is in the address: `/live?machine=<key>`, where the key is the machine's id
  when its rows carry one — a new label does not break a link — and its label when they carry
  none, or several, two machines given one label being one part (SE-C220); `unnamed` is the rows
  that name none, and All is the address without `machine=`. Choosing a tab pushes a history entry,
  as opening a session does (SE-C189) and the Overview's scope (OV-C30), so Back returns to the tab
  before; choosing the tab already chosen adds nothing. A link selects its tab. Every address the
  screen writes on `/live` keeps `machine=`, first, with the rest of the address as it was: opening
  a session from the list, a search hit, the open session's tabs, Find, closing Find, and Back to
  the list. So a session opened from a machine's tab goes back to that tab, by the button or the
  browser, and on a desk the list beside an open session stays on its tab. The open session is
  drawn as ever. *Why:* the address holds the state (`ui/src/lib/routes.ts`); a refresh, a link or
  Back that lost the tab would put the reader on a list they did not choose. *Regression:* no.
  *Source:* `ui/src/screens/Sessions/index.tsx`, `ui/src/screens/Sessions/Strip.tsx`;
  `ui/src/model/session.ts` (`machineParam`, `withMachine`).
- **SE-C229** An address whose `machine=` names a machine the list does not hold now — it went
  offline, or the link is old — shows All, replacing its own history entry, and says so in one
  line over the list: "That machine has no sessions listed now, so this is all of them." It never
  draws an empty tab or an error. The list read must be one made since the machine was asked for,
  and the check is made at every read, so a machine that leaves while its tab is open leaves it the
  same way. An old link naming both a machine and a session that are gone lands on All and says
  both sentences, the session's first (SE-C191). With one machine there are no tabs, and an address
  naming that machine is kept as it is and says nothing. *Why:* a machine's sessions leave the list
  when its agent disconnects, and a link to it outlives that. *Regression:* no. *Source:*
  `ui/src/screens/Sessions/index.tsx`; `ui/src/model/session.ts` (`listsMachine`,
  `MACHINE_UNLISTED`).
- **SE-C230** The row of tabs is one row that scrolls sideways inside the list, with Carbon's own
  overflow chevrons, and never wraps or widens the list or the page at 300 to 360 px. It stays at
  the top of the list as the list scrolls under it. A long label is cut short with an ellipsis, as a
  machine's heading is (SE-C225), so one machine's tab never takes the whole row. On a coarse
  pointer each tab is a 48 px target (SE-C205). Choosing another tab shows it from its top, not
  from where the last tab was left. *Why:* Carbon caps its tabs at one 64 px row, so a wrapped second
  row would be out of reach on a phone (Northstar `29472eb`); a label is whatever the machine's agent
  was given. *Regression:* no. *Source:* `ui/src/screens/Sessions/Sessions.module.scss`
  (`.machineTabs`); `scripts/sessions-checks.mjs` measures it at 360 px.

### Navigation

- **SE-C189** The open session lives in the URL as `?session=<id>`, pushed when a session opens and
  removed when the list returns, so a refresh lands back on it and a link opens it; `/live` and
  `/live?session=<id>` keep answering. *Why:* on a phone a refresh happens every time the page is
  backgrounded long enough. *Regression:* yes — 426a197 (the parameter was only ever cleared, never
  set, so the refresh, the whole point, was broken). *Source:*
  `assets/live.html:1140-1145, 1397-1401, 1681-1689`; plan, "Who opens them".
- **SE-C190** The browser's Back moves between the list and a session rather than leaving the page,
  and a URL that names a session reopens it from the router's list. *Why:* Back is how a phone
  leaves a session. *Regression:* yes — 3ffb04f (the first cut threw on the deep-link path only);
  checks `opening a session does not throw`, `going back to the list does not throw`. *Source:*
  `assets/live.html:1545-1552`, `scripts/check-live-ui.js:276-282`.
- **SE-C191** A URL that names a session the router does not list lands on the list and says
  so. *Why:* a link to one entry lands on it, or says why it cannot (Northstar `docs/13` §3,
  §10). *Regression:* no. *Source:* `assets/live.html:1549-1551, 1684-1688`. *Today:* broken — the
  list is shown with nothing said.
- **SE-C192** The screen is keyed on the session id: a draft, a half-answered form, the notices, the
  arrow history and the permission card belong to one session and never follow the reader to
  another. *Why:* a screen kept mounted across a change of route saved drafts onto the next project
  (Northstar `940a783`). *Regression:* yes — Northstar 940a783. *Source:*
  `assets/live.html:1152-1155, 1161-1163`; plan, "Data". *Today:* broken — the composer's draft
  survives a switch to another session through a tab or a hit, where Enter sends it.
- **SE-C193** Every async outcome — the totals, a switch's result, an answer's, an archive's, a side
  answer, a send's echo — is tagged with the session it was for and applied only there; one that
  comes back after another session opened is dropped from view. *Why:* results that land in
  whichever session is open draw one session's facts under another and post its controls to the
  wrong one. *Regression:* no. *Source:*
  `assets/live.html:1185-1209, 1330-1347, 1355-1359, 1587-1608, 1639-1659`; plan, "Data". *Today:*
  broken — only the prompt poll and Find's sequence check do; everything else lands in whichever
  session is open, a side answer up to minutes later.
- **SE-C194** Opening a session empties the previous session's feed at once and starts this one at
  the top, its rows arriving as its stream replays them; its header says "connecting…" (SE-C61), the
  card and the notice are cleared, and its strip and facts are drawn. *Why:* another session's rows
  go at once. *Regression:* no. *Source:* `assets/live.html:1135-1184`.
- **SE-C195** Going back to the list closes the stream, clears the session's header, strip, facts,
  composer and notice, and puts the list back where the reader left it, its scroll position restored
  once the rows are back. *Why:* coming back must not dump the reader somewhere else, and a position
  restored before the rows return has nothing tall enough to scroll. *Regression:* no. *Source:*
  `assets/live.html:1136-1138, 1387-1404`.
- **SE-C196** The heading says where the reader is: the screen's name on the list, the project's
  name with a session open, beside it the machine it runs on when a hosted router names one
  (SE-C221), and "Find" in Find. *Why:* on a phone it is the one line that says where you are.
  *Regression:* no. *Source:* `assets/live.html:374, 1151, 1393, 1439, 1454, 1459`.

### Layout and keyboard

- **SE-C197** The screen is a fixed-height column pinned to the dynamic viewport — header, strips,
  one scrolling pane, composer last — so the composer cannot drift and nothing reflows when the
  iPhone's URL bar shows or hides. *Why:* a body-scrolled page with a sticky composer sat mid-screen
  and lurched with the URL bar, which `vh` does not follow and `dvh` does. *Regression:* yes —
  d8e7f52. *Source:* `assets/live.html:25-37`; plan, "One app in Northstar's shell".
- **SE-C198** Whichever pane is visible — the list, the feed or Find — owns the scroll, and its
  scroll never chains to the page. *Why:* the panes are interchangeable, and a chained scroll moves
  the whole column. *Regression:* no. *Source:* `assets/live.html:73-77`.
- **SE-C199** The header clears the top safe area and the composer the bottom one. *Why:* the page
  runs edge to edge (`viewport-fit=cover`), and the shell's fixed header ignores safe areas until
  padding is added. *Regression:* no. *Source:* `assets/live.html:3, 42, 279`; plan, "Carbon traps
  already known".
- **SE-C200** The on-screen keyboard never covers the composer: the layout resizes with it, and on
  iOS, which overlays the keyboard instead, the column is pinned to the visual viewport, once per
  frame; the newest message stays in view as the keyboard opens. *Why:* iOS ignores
  `interactive-widget`, so `dvh` alone left the composer underneath the keyboard. *Regression:* yes
  — d8e7f52. *Source:* `assets/live.html:3, 1662-1679`.
- **SE-C201** On a desk from 1,056 px the list sits beside the feed, and below that one pane shows
  at a time. With both on screen, the list keeps refreshing in full and marks the open
  session. *Why:* Decision H. *Regression:* no. *Source:* `assets/live.html:73-79, 619-623`; plan,
  "Decisions".
- **SE-C202** Nothing scrolls the screen sideways at 360 px: the header's items shrink and
  ellipsise, and the status never crushes the heading to nothing. *Why:* a phone gets every fact a
  desk does (Northstar `docs/13` §7). *Regression:* no. *Source:*
  `assets/live.html:45-46, 59-60`. *Today:* broken — the header's status never shrinks or wraps, so
  a long one ("Waiting for you · background shell running · 10h · 1 queued") squeezes the heading to
  nothing and runs off the edge of a 375 px phone (found by reading).
- **SE-C203** Every control is a real button or link, reachable by keyboard and named for assistive
  technology: a list row, a shelf row's Restore, a tab and its Archive, the thinking and fold
  toggles, a hit, a notice's dismiss and Back. *Why:* a keyboard and a screen reader only reach real
  controls. *Regression:* no. *Source:*
  `assets/live.html:373, 438-442, 657-672, 736-739, 808-816, 982, 1517-1518`; plan, "Where each part
  goes". *Today:* broken — rows, hits and the toggles are `div`s and `span`s with click handlers,
  and Back is named "‹".
- **SE-C204** No fact or explanation is reachable only by hover: what "live" and "view" mean, what a
  tab's session is doing, what each switcher does, what Find searches ("everything this router has
  recorded"), what btw does ("a quick side question without interrupting the session"), what Stop
  and a tab's Archive do — each is visible text, or one tap away, next to the control it
  concerns. *Why:* a phone and a keyboard cannot hover. *Regression:* no. *Source:*
  `assets/live.html:372, 376, 401, 664-665, 1017, 1020, 1218-1220, 1637`; plan, "What a literal port
  would inherit". *Today:* broken — each is a `title` tooltip.
- **SE-C205** On a coarse pointer every field is 16 px and every target 48 px. *Why:* iOS zooms into
  a field under 16 px and never zooms back out. *Regression:* no. *Source:*
  `assets/live.html:26, 85-86, 245-246, 283-284, 368`; plan, "Where each part goes". *Today:* broken
  — the composer and Find's field are 15 px, the Other field 13 px and the switchers 12 px, and tabs
  and scope buttons are smaller targets.
- **SE-C206** The header's status is announced to assistive technology when what the session is
  doing changes, never at each once-a-second heartbeat whose duration ticks. *Why:* a live region
  that speaks every second is noise. *Regression:* no. *Source:* `assets/live.html:377, 556-566`.
- **SE-C207** Every text meets WCAG AA contrast in both themes, the person's bubble included. *Why:*
  the plan's contrast pass. *Regression:* no. *Source:* `assets/live.html:9, 18, 178-184`; plan,
  "What a literal port would inherit". *Today:* broken — by the plan's readers' measurements, white
  on the dark theme's bubble is 2.52:1, and the light theme's faint text 2.7–3.1:1.

### Contract tests

- **SE-C208** Every check in `scripts/check-live-ui.js` is ported to Vitest over the React-free
  modules, each seen failing before it is trusted. *Why:* several checks in the old harness could
  never fail. *Regression:* yes — eedc5a8's fix (`scripts/check-live-ui.js:33-39`: everything after
  a check's first `await` never ran, and the check reported ok),
  `scripts/check-live-ui.js:327-330`. *Source:* `scripts/check-live-ui.js:1-13, 33-45, 327-330`.
- **SE-C209** Checks that drive one screen run as one chain, each awaited to its end, and a test
  waits for its condition rather than for one tick. *Why:* checks started together move state out
  from under each other, and a query library notifies on its own timer (Northstar
  `d834fd6`). *Regression:* no. *Source:* `scripts/check-live-ui.js:33-45`.
- **SE-C210** Anything the screen starts and does not await — a poll, a redraw — that rejects fails
  the run. *Why:* its failure is the screen's failure, not a crash that hides the
  summary. *Regression:* no. *Source:* `scripts/check-live-ui.js:46-48`.
- **SE-C211** Time is moved, not waited for: the tests drive the clock forwards and back, and fire
  the screen's own timers — the five-second list, the two-second prompt poll, the one-second
  watchdog — when they choose. *Why:* ten real seconds per silence check is a suite nobody
  runs. *Regression:* no. *Source:* `scripts/check-live-ui.js:188-202`.
- **SE-C212** Fixtures come in both shapes, the current router's keys and an older router's, whose
  rows must render exactly as before, and the router's clock is set off the test machine's so that a
  duration measured on the wrong clock shows. *Why:* a check fed only one shape never sees the other
  break. *Regression:* no. *Source:* `scripts/check-live-ui.js:9-12, 239-266`. *Under decision J:* the console ships inside the router it reads, so the older-router case here never reaches it; the rest of the rule stands, and the phones still meet older routers.
- **SE-C213** A fake `EventSource` lets a test speak as the router — a row, a named status, the
  connection's error, the router's `error` event — and frames the wire as a browser does: an event
  ends at a blank line, `event:` names it and one without a name is a message, `data:` lines join
  with newlines, a line opening with ":" is a keep-alive comment, an event cut off before its blank
  line is dropped, and multi-byte characters survive. *Why:* a byte splitter that swallowed the
  blank line gave the iPhone an evening of blank feed, and one that built text a byte at a time
  turned "…" into "â¦" on Android. *Regression:* yes — iOS
  `testRawBytesBecomeFramesEmptyLinesIncluded`, Android
  `multi-byte characters survive the splitter`. *Source:*
  `scripts/check-live-ui.js:112-125, 887-906`. *Phones:* iOS
  `testSSEFramesAssembleAndKeepAlivesAreIgnored`, `testRawBytesBecomeFramesEmptyLinesIncluded`;
  Android `sse frames assemble and keep-alives are ignored`,
  `raw bytes become frames, empty lines included`, `multi-byte characters survive the splitter`,
  `frames arrive in order and the flow ends with the router's stream` (EventsTest.kt).
- **SE-C214** The built-in wire — three rows and nine statuses, one per activity and one from an
  older router — replays through the real stream handling on every run. Every row lands, and after
  each status the header's class is a known tone (the router's, when it chose one) or stale, its
  words are not empty, and they read exactly "Waiting for you · background shell running · 10h",
  "Thinking", "Running Bash · 2m", "Bash wants to run · 1m · 1 queued", "2 background agents running
  · 5m", "No progress · 12m", "API error: 529 Overloaded · 1m", "Session ended mid-turn · 3h ago"
  and "Waiting for you · 15m". *Why:* synthetic checks only ever see the payloads they were written
  with, and the replay is what the router really sends — which once skipped every status, so no
  captured status ever reached the header. *Regression:* yes —
  `scripts/check-live-ui.js:884-885`. *Source:* `scripts/check-live-ui.js:877-986`. *Under decision J:* the console ships inside the router it reads, so the older-router case here never reaches it; the rest of the rule stands, and the phones still meet older routers.
- **SE-C215** A capture of a real stream, handed to the test run, replays the same way, and a path
  that names no file fails rather than skipping. *Why:* it is a mistake to hear about, not a replay
  to skip in silence. *Regression:* no. *Source:* `scripts/check-live-ui.js:988-996`.
- **SE-C216** The Markdown rules and the list's grouping rules, a hosted router's list headed per
  machine included, are pinned by fixture files that the web and both phones test against
  (`markdown-blocks.json`, `session-groups.json`). *Why:* the three clients disagreed on numbered
  lists, joined paragraphs, quotes, unterminated fences and heading levels, and one fixture is what
  keeps them from drifting again; lists and the per-machine heading were decided in the phones'
  favour on 2026-10-05, and the rest in the web's. *Regression:*
  no. *Source:* `assets/live.html:675-744, 749-794`; plan, "The order of work". *Phones:* iOS
  `testMarkdownBlocksMatchTheWebView`, `testSessionsGroupLikeTheWebView`; Android
  `markdown blocks match the web view`, `sessions group like the web view`.
- **SE-C217** No module-level name shadows a browser global the screen uses — `history`, `location`,
  `name`, `status`, `top`, `self`, `parent`, `origin`, `screen`, `event`, `find` — and lint enforces
  it. *Why:* a local named `history` made `pushState` throw inside `openSession`, so every card
  silently stopped opening; this class of bug has no symptom where the mistake is. *Regression:* yes
  — 4a48211; check `a local does not shadow a browser global the page uses`. *Source:*
  `assets/live.html:451-453, 1415-1416`, `scripts/check-live-ui.js:268-274`.
- **SE-C218** The screen loads, opens a session — by a click and from a URL — and goes back, without
  throwing, and a crash in it leaves the shell alive with an error, never a blank page. *Why:* every
  session once opened blank, and a throw on the deep-link path alone passed every
  click-through. *Regression:* yes — 3fe8c5d, 3ffb04f; checks `script evaluates`,
  `opening a session does not throw`, `going back to the list does not throw`. *Source:*
  `scripts/check-live-ui.js:204, 276-282`; plan, "What we take from Northstar".

## Machines

A hosted router (`agent-router serve --server`) serves the Claude Code sessions of developers'
machines, each running the Agent Router agent, which dials out to the router
([hosting the machine agent](hosting-machine-agent.md), commit `60cf5f8`). The Machines screen, at
`/machines`, is where a machine is connected and where the machines the router has heard from are
seen. It was asked for on 2026-10-07: "Machines should be first level right after the Live sessions
in the menu." Until then the installer's commands showed only in the Sessions list's empty state,
which a router with one machine connected never draws, so its owner could not find them. A local
router takes no machines. Line numbers are as of commit `48533db`.

### Where it is

- **MA-C1** The screen is at `/machines` and names itself in the tab's title, "Machines · Agent
  Router" (SH-C7). On a hosted router the rail lists Machines directly after Sessions, with the
  machine icon (SH-C39), at both widths, marked as current on its screen; a local router's rail
  does not, and nor does the rail before `/api/auth` has said which kind of router this is
  (ST-C101). The palette's "Go to" lists it on every router, as it lists every Settings page, and
  the address says why on a local router (MA-C7). *Why:* the owner's request; a local router takes
  no agents, so its rail would offer a page with nothing on it, and a link drawn before the router
  has answered and then taken away would move the rail under the reader. *Regression:* no.
  *Source:* `src/auth.rs:356` (`mode`); `ui/src/app/Shell.tsx`, `ui/src/lib/routes.ts`,
  `ui/src/screens/Palette/hits.ts`.

### Connecting a machine

- **MA-C2** On a hosted router the screen opens with "Connect a machine", whatever its list holds:
  "Run this on a machine to connect it — it asks for this router’s password.", then the
  installer's command for each platform at the page's own origin, each labelled and with its own
  copy button — "macOS and Linux", `curl -fsSL <origin>/agent/install.sh | sh`, and "Windows
  (PowerShell)", `irm <origin>/agent/install.ps1 | iex` — and last "With VS Code’s code command on
  the machine, the script also adds the Agent Router extension, so its sessions can be typed into
  from here and from the phone; without it, they can only be followed." It is the console's one
  place for the commands: a hosted router's empty session list links here (SE-C187). *Why:* the
  router serves the installer itself, without a password, so the page gives the command to run
  rather than name a program the reader has nowhere to get; and a machine is connected while
  others are listed, so the commands cannot wait for an empty list. *Regression:* no. *Source:*
  `src/downloads.rs:64-72` (`install_sh`, `install_ps1`), `src/downloads/install.sh:1-13`;
  `ui/src/components/InstallCommands.tsx`.

### The machines

- **MA-C3** Under the commands, "Seen since this router started" lists every machine
  `GET /api/machines` names — every machine that has connected since the router last started,
  online or not — read every five seconds on the console's one clock (SH-C15). The online ones come
  first, then the rest, each part by label in the order the session list heads its machines
  (SE-C220), and over the list it says how many are online and how many are not ("2 online · 1
  offline"). Each machine shows its label, its platform in words (macOS, Linux, Windows, and one
  the console does not know as the agent sent it), and whether it is online, with a dot and in
  words, and for how long, measured from `since_ms`: "online for 3h", "offline for 2d". A machine
  whose label is blank is named by the start of its id. *Why:* the owner reads a page at a glance
  rather than across a wide table, and which machines are connected decides which sessions a
  hosted router can reach (SE-C223). *Regression:* no. *Source:* `src/machines.rs:229-240`
  (`Seen`), `src/machines.rs:396-400` (`seen`), `src/agent.rs:426-427` (`--label`, else the host
  name; `std::env::consts::OS`).
- **MA-C4** Each machine says whether its agent runs the router's own build, its `build` compared
  with `/_router/health`'s: the same build is said quietly, "Same build as this router"; another is
  said in the warning colour, with what updates it: "Runs a different build from this router — run
  the installer on it again to update it." Until the router's health has answered, nothing is
  said of any machine's build. *Why:* a hosted router is upgraded on its own, and each machine's
  agent stays on the build it was installed with until the installer runs there again; and a
  build is `tree:` and forty hex digits, which nobody compares by eye. *Regression:* no. *Source:*
  `build.rs:80-101` (`tree_id`), `src/api.rs:1716`, `src/machines.rs:229-240`.
- **MA-C5** With no machine seen, one line under the commands says "No machine has connected
  yet." — the sentence a hosted router's empty session list says (SE-C187) — and no empty list is
  drawn. *Why:* an empty list says which empty it is (SH-C22, Northstar `docs/13` §8).
  *Regression:* no. *Source:* `ui/src/model/machine.ts`.
- **MA-C6** A failed read blanks nothing: the machines already read stay, under "Showing what was
  read 12s ago: the last read failed — " and the router's words; a read that never succeeded says
  "Could not read the machines" and the router's words, never that no machine has connected; and a
  reply that is not a list is a failed read. The commands stay in every case. *Why:* a blip is not
  an empty router, and an empty list and an unreadable one are different facts (SH-C14, SH-C22).
  *Regression:* no. *Source:* `ui/src/data/held.ts`, `ui/src/data/machines.ts`.

### A local router

- **MA-C7** On a local router, `/machines` reached by its address says "Machines connect to a
  hosted router. This router runs on this machine and serves its sessions itself, so there is
  nothing to connect.", offers no command and never reads `/api/machines`. Until `/api/auth` has
  answered it says it is asking, and a failed answer says "Could not read what kind of router this
  is" with the router's words. *Why:* a local router takes no agents and does not answer
  `/api/machines`, and an address it does not serve says why rather than drawing a blank, as a
  hosted router's Settings address does (ST-C103). *Regression:* no. *Source:* `src/hosted.rs:64`,
  `src/auth.rs:356`.

## Library

A hosted router keeps a Library of skills, commands and subagents, and every connected
machine's agent keeps its `~/.claude` in step with it ([the Library](library.md), AGEN-0042).
The Library screen, at `/library`, is where the owner adds a version, reads what each item
holds and where each version came from, decides what waits for approval, rolls back, archives,
and tells the machines to sync now. The Machines screen says where each machine's Library
stands. Built 2026-10-08 as AGEN-0051, against the router of commits `8f940d0` to `15f3dda`.

### Where it is

- **LB-C1** The screen is at `/library` and names itself in the tab's title, "Library · Agent
  Router" (SH-C7). On a hosted router the rail lists Library directly after Machines. A local
  router's rail does not, and its screen says why: "The Library is a hosted router’s…"; nothing
  is read and nothing is offered. The palette's "Go to" lists it on every router, as it lists
  Machines. *Why:* a local router keeps no Library (`library::api::not_offered`). *Regression:*
  no. *Source:* `ui/src/screens/Library`, `ui/src/app/Shell.tsx`, `ui/src/screens/Palette/hits.ts`.

### The items

- **LB-C2** Each item shows its kind, its name, what it says it is for, its live version ("v3
  live", or "none live"), where its versions come from, how many wait for approval, and whether
  it is archived. There are three places versions come from:
  - "uploaded and published here";
  - "imported by the source <key>";
  - "comes with the router’s build".

  *Why:* at a glance, which items want a decision. *Source:* `GET /api/library`
  (`src/library/api.rs`, `list`).
- **LB-C3** Items are listed by kind (skills, commands, subagents), then by name. An empty
  Library says so, and says what adding one does: "The Library is empty. Add a skill…".
  *Source:* `ui/src/model/library.ts` (`itemOrder`, `EMPTY`).
- **LB-C13** A failed read is never an empty Library. The last good list is kept and marked
  with its age, and a Library never read says it could not be read, in the router's words
  (SH-C22). *Regression:* no; it is the rule every screen keeps (MA-C6).

### An item

- **LB-C4** An item opens at `/library?item=<kind>/<name>`. It shows:
  - the file Claude Code reads (a skill's `SKILL.md`, or the command's or subagent's one
    file) of its live version (or its newest, while none is live), without its frontmatter,
    rendered as Markdown and never as HTML (SE-C90);
  - its files, with their sizes and the executable mark. A file opens as text.

  The router serves files as plain text under a sandbox policy, as a second line of defence.
  *Source:* `src/library/api.rs` (`file`).
- **LB-C5** Its versions, newest first, each show:
  - its number and its state: "live", "waits for approval", "rejected" or "superseded";
  - where it came from, in words: "uploaded from the console", "imported from <repository>
    at <commit>, <path>", "published with the token <name>", "published from <machine> by a
    Claude Code session" (with a link that opens that session), or "built into the router";
  - when.

  *Why:* the owner should see which session wrote a change. *Source:* `src/library/store.rs`
  (`Provenance`).
- **LB-C6** What can be done to each version:
  - one that waits offers "Approve" and "Reject";
  - a superseded one offers "Make live again", which is a rollback;
  - a live one and a rejected one offer nothing.

  Each act is sent once. What it did, or the router's refusal in its words, shows beside the
  item, and the Library is read again. *Source:* `POST …/versions/{n}/live`, `…/reject`.
- **LB-C7** "Compare with the live one" shows how a version's main file differs from the live
  version's, line by line, added and removed lines marked. *Source:* `ui/src/model/library.ts`
  (`diffLines`).
- **LB-C8** "Archive" asks once more, "Take it off every machine", before it acts. An archived
  item says so, and says it is on no machine until a version of it goes live again.
  *Source:* `POST …/archive`.

### Adding a version

- **LB-C9** "Add to the Library" takes a kind, a name, and either a skill's folder or one file
  (a .zip, a .tar.gz or one .md).
  - **A folder** is packed in the browser into the tar the router reads, each file by its path
    within the folder, with a machine's clutter (`.git`, `.venv`, `__pycache__`,
    `node_modules`, `.DS_Store`, `._*`) left out, as the router leaves it out.
  - **The name** is the folder's or the file's, unless one is typed, and a name the Library
    cannot take is said before anything is sent.
  - **What happened** is said: "Published the skill bench as version 3: live, and every
    connected machine is told now", "Nothing new…", or the router's refusal in its words (a
    credential, a missing `SKILL.md`, an item that comes from a repository).

  *Source:* `ui/src/model/library.ts` (`packFolder`), `src/library/bundle.rs` (`from_upload`).
- **LB-C10** "Sync every machine now" tells every machine and lists what each said:
  - synced (and how many items changed);
  - its agent predates the Library (run the installer again);
  - it takes no part;
  - it is offline (it syncs when it connects again);
  - it failed, and why.

  *Source:* `POST /api/library/sync` (`src/library/push.rs`).

### On the Machines screen

- **LB-C11** Each machine says where its Library stands:
  - "Library in step";
  - "Library behind on <n> items";
  - "<n> items could not be placed";
  - "in step; it put back <items> over a copy changed there";
  - "Its agent predates the Library — run the installer on it again";
  - "Takes no part in the Library";
  - "Nothing heard of its Library yet".

  What wants something done is marked. A machine offline when a version went live reads as
  behind. *Source:* `/api/machines` rows' `library` (`src/library/push.rs`, `standing`).
- **LB-C12** An online machine offers "Sync its Library now", which tells that machine alone and
  shows its answer beside it. *Source:* `POST /api/library/sync` with `{machine}`.

## Settings

What Settings must do: pages under `/settings`, one per section, keeping every rule today's
settings dialog keeps. Northstar §5 (refusals beside their control, in the server's words)
and §9 (edit where you read; a control that could only be refused is not offered) bind
throughout, and so do the owner's preferences: actions visible at rest, never behind a
disclosure, and each explanation next to the field it concerns.

### Pages and tabs

- **ST-C1** Settings is pages under `/settings`: one page header with the sections as tabs
  that are real links — Accounts, Claude Code, Network, Password, Notifications, Router — so
  each section has its own URL, a reload stays on it and Back moves between them; `/settings`
  alone opens Accounts. *Why:* Decision E; today's settings are a modal with no address of
  its own. *Regression:* no. *Source:* `docs/web-console-carbon.md:184-188`,
  `docs/web-console-carbon.md:205-206`, `docs/web-console-carbon.md:551-552`,
  `docs/web-console-carbon.md:634`, `assets/index.html:526-644`.
- **ST-C2** Every tab can be reached on a 360 px phone. *Why:* Carbon's tabs are capped at
  one 64 px row, so wrapped tabs could not be reached on a Northstar phone, and six sections
  do not fit one row there. *Regression:* yes — Northstar `29472eb`. *Source:*
  `docs/web-console-carbon.md:330`, `docs/web-console-carbon.md:609-610`.
- **ST-C3** Every action on a Settings page is a visible button at rest — never inside a
  collapsed panel, an overflow menu or a hover — and a form a button opens, opens in place.
  *Why:* the owner once missed a delete button inside a collapsed panel; Use, Edit and Remove
  are always visible. *Regression:* no. *Source:* `docs/web-console-carbon.md:371-372`,
  `docs/web-console-carbon.md:376`, `docs/web-console-carbon.md:385-386`,
  `assets/index.html:563-564`.
- **ST-C4** Each explanation is visible text next to the field or switch it concerns, never
  only a `title`. *Why:* the owner's standing preference, and Carbon's `Tag` drops `title`.
  *Regression:* no. *Source:* `assets/index.html:545-546`, `assets/index.html:553-555`,
  `assets/index.html:577-579`, `assets/index.html:582-584`, `docs/web-console-carbon.md:323`,
  `docs/web-console-carbon.md:377-378`.
- **ST-C5** Each Settings page reads what it shows when it opens — Accounts with every
  credential probed, Claude Code its setup and wrapper state, Notifications its push state,
  Router the health — and re-reads after each of its own writes; auth and accounts are also
  re-read every 30 s. *Why:* "Resolving every credential is not free, so it is done when the
  panel is actually open." *Regression:* no. *Source:* `assets/index.html:942-950`,
  `assets/index.html:2503-2509`, `assets/index.html:2516`, `docs/web-console-carbon.md:237-238`.
- **ST-C6** `/api/accounts?probe=1` is asked only while the Accounts page is open, and every
  other read of `/api/accounts` resolves no credential. *Why:* resolving every credential on
  every 30-second poll repeats a lookup per account for nothing. *Regression:* no. *Source:*
  `assets/index.html:947-948`, `assets/index.html:2506`, `assets/index.html:2516`,
  `src/api.rs:1326-1376`, `docs/web-console-carbon.md:443`. *Today:* broken — the router
  ignores `probe` and resolves every credential on every call (AGEN-0015). *Amended:* the Overview also makes ONE probed read per load, the read ST-C7 needs to open Settings by itself; every other read outside the Accounts page resolves no credential (185f609, and the console's OpensSettings spec).
- **ST-C7** At most once per load, and only from the Overview, the console opens Settings →
  Accounts by itself when nothing can sign or when an account that could sign reports a
  credential that does not work. *Why:* "Nothing configured, or a credential that stopped
  working, is worth interrupting for"; once, so a reader who leaves it is not dragged back.
  *Regression:* no. *Source:* `assets/index.html:1975-1985`, `assets/index.html:2502`.
- **ST-C8** On a hosted router that interruption counts only the accounts it could sign
  with — token accounts, and whichever account is active — so a Claude Code login or a
  profile copied in with a Mac's store interrupts only while it is the one in use. *Why:*
  those rows can never be ready on a hosted router, and would otherwise open Settings on
  every load. *Regression:* yes — check "a hosted router opens the settings for a Claude Code
  login only while it is the one in use". *Source:* `assets/index.html:1975-1981`,
  `scripts/check-dashboard-ui.js:455-494`.

### Outcomes and refusals

- **ST-C9** A write in Settings shows its own progress and outcome beside its own control:
  working while it runs, then the server's result or refusal in the server's words. *Why:*
  Northstar §5. *Regression:* no. *Source:* `docs/web-console-carbon.md:380-381`,
  `assets/index.html:615`, `assets/index.html:2096-2102`,
  `../northstar/docs/13-console-contracts.md:253-254`. *Today:* broken — every Claude Code
  action writes into one shared line under the section.
- **ST-C10** A refusal is never shown as success: a password the router refused never reads
  "set", a push key never "saved", a setup switch never "done". *Why:* those routes answer
  refusals as HTTP 200 with `{error}`, which today's `post()` resolves as success.
  *Regression:* no. *Source:* `assets/index.html:815-820`, `assets/index.html:2339-2340`,
  `assets/index.html:2466`, `assets/index.html:2099`, `src/auth.rs:385-396`,
  `src/notify.rs:1124-1145`, `src/setup.rs:343-344`, `docs/web-console-carbon.md:437`.
  *Today:* broken — all three report success for a refusal (AGEN-0014).
- **ST-C11** A rule the router enforces is checked before sending, with the router's own
  numbers and sentences, beside the field it concerns: a password of at least 8 characters,
  an identity that looks like an email address, a credential that starts `sk-ant-` and is not
  a `sk-ant-sid` session key, a session key that starts `sk-ant-sid`, and a team id, key id
  and `.p8` given together. *Why:* Northstar §5 — asked before the request, not after its
  refusal, with the server's own minimum. *Regression:* no. *Source:* `src/auth.rs:389-392`,
  `src/api.rs:1256-1264`, `src/api.rs:1293-1310`, `src/quota.rs:73-81`,
  `src/notify.rs:1130-1133`, `../northstar/docs/13-console-contracts.md:264-265`.
- **ST-C12** A control the router could only refuse is not offered at all, and where the
  reason matters its place says why in words. *Why:* Northstar §9 — an offered control that
  always refuses is worse than none. *Regression:* no. *Source:*
  `../northstar/docs/13-console-contracts.md:370-376`, `assets/index.html:1965-1970`,
  `assets/index.html:2324-2326`, `assets/index.html:2369`.
- **ST-C13** Every switch shows the state the router reports, read back after the write,
  never a state flipped in advance. *Why:* a switch showing what was asked instead of what is
  true lies until the next read. *Regression:* no. *Source:* `assets/index.html:2096-2102`,
  `assets/index.html:2374-2376`, `docs/web-console-carbon.md:380-384`.
- **ST-C14** An editor opens on its button, saves on Enter, cancels on Escape, sends only
  what changed, stays open with the refusal and everything typed when it is refused, and
  closes once the saved value reads back. *Why:* Northstar §9. *Regression:* yes — the
  regression recorded at `../northstar/docs/13-console-contracts.md:368-369` (an editor that
  closed on open because the draft equalled the value). *Source:*
  `../northstar/docs/13-console-contracts.md:366-369`, `assets/index.html:2148-2172`,
  `assets/index.html:2347`. *Today:* broken — Enter does nothing in the account editor or the
  add form; only the password's second field submits on Enter.
- **ST-C15** A poll never rebuilds a Settings page under the reader: focus, an open editor, a
  half-typed value and a shown outcome survive it, and the password and network forms redraw
  only when their own facts change. *Why:* "a half-typed password must not be swept away
  because the machine picked up a new address on the network". *Regression:* yes — the
  comment at `assets/index.html:2425-2426`. *Source:* `assets/index.html:2422-2429`,
  `assets/index.html:1935-1944`, `assets/index.html:2550`. *Today:* broken — the accounts
  table is rebuilt from HTML on every five-second refresh, so a focused Use, Edit or Remove
  button loses focus.
- **ST-C16** Every secret field is a password input with `autocomplete="new-password"` and
  spellcheck off, the `.p8` box has spellcheck off, and the identifier fields (team id, key
  id, bundle id, a project path) have autocomplete and spellcheck off. *Why:* a browser must
  not autofill a saved login into a credential field, and a spellchecker must not ship a
  secret anywhere. *Regression:* no. *Source:* `assets/index.html:543`,
  `assets/index.html:550`, `assets/index.html:575`, `assets/index.html:608`,
  `assets/index.html:2328-2329`, `assets/index.html:2460-2463`.
- **ST-C17** A secret is never sent back to the page: a stored credential, session key or
  push key shows as held — dots in its field's placeholder and words that say it is stored
  and that leaving the field blank keeps it — and an empty field means keep. *Why:* "Show
  that a secret is held without ever sending it back: dots in the placeholder, so the field
  looks filled but submits empty, which means 'keep'." *Regression:* no. *Source:*
  `assets/index.html:2018-2035`, `assets/index.html:2463`, `src/api.rs:1366-1369`,
  `src/accounts.rs:403-407`, `src/notify.rs:1095-1101`.
- **ST-C18** Removing an account, the password, the push key or an account's session key,
  and turning the network off, each ask first in a danger modal that names what goes and
  what follows; no other Settings action asks. *Why:* a reader on another machine can lock
  itself out with two of them, and the rest of Settings is switches that switch back.
  *Regression:* no. *Source:* `docs/web-console-carbon.md:374`,
  `docs/web-console-carbon.md:385-386`, `docs/web-console-carbon.md:448`,
  `assets/index.html:1998`, `assets/index.html:2137-2147`, `assets/index.html:2350-2353`,
  `assets/index.html:2366-2378`, `assets/index.html:2473-2475`. *Today:* broken — Remove
  password, Remove key, Turn off and Remove stored key act on one click, and Remove account
  asks through a native `confirm()`.

### Accounts: the list

- **ST-C19** Each account is a card carrying every fact a desk row has: active (or a Use
  button), email or label, org, plan, source, a "usage key" tag when a session key is
  stored, the credential's state, when it expires, and Edit and Remove (or why it cannot be
  removed). *Why:* "One card per account rather than nine columns: the row never truncates,
  at any width." *Regression:* no. *Source:* `assets/index.html:304-315`,
  `assets/index.html:1946-1971`, `docs/web-console-carbon.md:371-372`.
- **ST-C20** A credential that was not checked reads "not checked", never ready or failed;
  a ready one names its kind; a failed one shows the router's whole reason, wrapped, in the
  error tone. *Why:* not asked and asked-and-fine are different facts (Northstar §2).
  *Regression:* no. *Source:* `assets/index.html:1959-1962`, `src/api.rs:1354-1372`,
  `../northstar/docs/13-console-contracts.md:36-47`. *Today:* broken — the reason is cut at 70
  characters with no way to read the rest.
- **ST-C21** Expiry reads "no expiry" for a credential that does not expire, a date for one
  that does, and "—" when it is not known. *Why:* no expiry and unknown are different facts.
  *Regression:* no. *Source:* `assets/index.html:1963-1964`.
- **ST-C22** The list's heading counts the accounts and names the active one, or says "none
  active". *Why:* exactly one account signs at a time, and the page says which at a glance.
  *Regression:* no. *Source:* `assets/index.html:1973-1974`, `src/accounts.rs:571-583`.
- **ST-C23** The "usage key" tag and the "built in" marker carry their explanations as
  visible text: a session key "used only to read usage", and a built-in account "discovered
  from the Claude Code login on this machine, so the router does not own it". *Why:* an
  explanation belongs beside what it explains, and Carbon's `Tag` drops `title`.
  *Regression:* no. *Source:* `assets/index.html:1955-1958`, `assets/index.html:1965-1970`.
  *Today:* broken — both explanations exist only in `title`.
- **ST-C24** Use switches the signing account in one click, with no confirmation and no
  restart asked for, and the list re-reads to show the new active account. *Why:* the router
  drops its cached credential so the switch takes effect on the very next request,
  mid-session, and it is undone the same way. *Regression:* no. *Source:*
  `assets/index.html:1987-1991`, `src/api.rs:1485-1489`.
- **ST-C25** A refused Use or Remove shows the router's sentence on that account's card,
  and the list re-reads. *Why:* activation and removal refuse as HTTP 500 with `{error}`.
  *Regression:* no. *Source:* `assets/index.html:1987-2001`, `src/api.rs:122-131`,
  `src/api.rs:1475-1481`, `src/api.rs:1496-1502`. *Today:* broken — the handlers have no
  catch, so a refusal is an unhandled rejection: nothing is shown and the list is not
  re-read.
- **ST-C26** On a local router the built-in Claude Code account offers no Remove and says it
  is built in; on a hosted router it is removable like any other. *Why:* locally the router
  refuses ("the default account is discovered automatically and cannot be removed") and would
  discover it again; a hosted router seeds none, so one that came with an imported store can
  go. *Regression:* yes — check "a hosted router opens the settings for a Claude Code login
  only while it is the one in use". *Source:* `assets/index.html:1965-1970`,
  `src/api.rs:1496-1502`, `scripts/check-dashboard-ui.js:463`,
  `scripts/check-dashboard-ui.js:479-485`.
- **ST-C27** On a hosted router a Claude Code login or a profile account offers no Use, and
  its card says it cannot sign there, in the router's sentence ("this account is a Claude
  Code login on a developer's machine, which a hosted router cannot read …"). *Why:* the
  router refuses to activate anything but a token there. *Regression:* no. *Source:*
  `src/api.rs:1475-1481`, `src/accounts.rs:141-144`, `docs/hosting-on-kubernetes.md:163-167`.
  *Today:* broken — Use is offered on those rows and its refusal is swallowed.
- **ST-C28** Remove's confirmation names the account, says its stored credential and
  session key are deleted, and, when it is the active account, names the account that will
  sign next: the oldest one left. *Why:* the router never leaves itself with nothing to sign
  with, so removing the active account activates the oldest remaining one. *Regression:* no.
  *Source:* `assets/index.html:1998`, `src/accounts.rs:585-606`.

### Accounts: editing

- **ST-C29** Edit opens its form inside the account's own card. *Why:* Northstar §9 — the
  page you read is the page you change; today the form opens in a box under the table.
  *Regression:* no. *Source:* `docs/web-console-carbon.md:373`, `assets/index.html:535-560`,
  `assets/index.html:2045-2046`, `../northstar/docs/13-console-contracts.md:351-352`.
- **ST-C30** The credential field appears only for a token account. *Why:* "Only a token
  account owns its credential; the others read Claude Code's" — the router refuses a
  credential for any other source. *Regression:* no. *Source:*
  `assets/index.html:2016-2017`, `src/api.rs:1293-1299`.
- **ST-C31** A Claude Code login's identity is shown and cannot be typed over, with the
  reason under it: locally, that it is read from the Claude Code login on this machine and
  refreshes on every request; on a hosted router, that it is Claude Code's own login from
  the machine the store was copied from, which a hosted router cannot sign with. *Why:*
  anything typed there would be overwritten on the next request. *Regression:* yes — check "a
  hosted router speaks of its deployment, not of this Mac, and a local one as it always did".
  *Source:* `assets/index.html:2036-2044`, `scripts/check-dashboard-ui.js:445-451`.
- **ST-C32** For a token or profile account the identity field says that setting it is what
  labels the account's traffic in the charts, because a long-lived token cannot report
  which account it belongs to. *Why:* without it, a token account's traffic has no identity.
  *Regression:* no. *Source:* `assets/index.html:2044`, `src/api.rs:1396-1398`.
- **ST-C33** Replacing a token keeps the account's id, so everything recorded against it
  stays attached, and the field says so. *Why:* "The account keeps its id, so everything
  already recorded against it stays intact." *Regression:* no. *Source:*
  `assets/index.html:545-546`, `src/api.rs:1266-1267`, `src/api.rs:1290-1292`.
- **ST-C34** A blank field in the editor means keep: a blank name, identity, credential or
  session key sends nothing for that field. *Why:* the router ignores an empty value, and a
  secret is never shown to be edited down. *Regression:* no. *Source:*
  `assets/index.html:2151-2161`, `src/api.rs:1273-1274`.
- **ST-C35** The editor sends the account's id and what the reader typed, never a copy of
  the row taken when it opened. *Why:* a Northstar dialog that captured its row when it
  opened re-sent stale data after a poll. *Regression:* yes — Northstar `28efc66`. *Source:*
  `assets/index.html:2148-2161`, `docs/web-console-carbon.md:601-602`.
- **ST-C36** A save that sends the account's fields and its session key reports each
  outcome on its own: a refused key is shown beside the session-key field while the name,
  identity or credential that were saved show as saved, and the list re-reads either way.
  *Why:* they are two requests, and the first one succeeds on its own. *Regression:* no.
  *Source:* `assets/index.html:2148-2171`, `src/quota.rs:73-81`. *Today:* broken — after the
  account edit succeeds, a refused key leaves only its refusal on screen and the list
  unrefreshed, under the old name.
- **ST-C37** Remove stored key is offered only while a session key is stored. *Why:* a
  control with nothing to act on is not offered; the router also drops its cached quota
  when the key goes. *Regression:* no. *Source:* `assets/index.html:2028`,
  `assets/index.html:2137-2147`, `src/quota.rs:66-71`.
- **ST-C38** The session-key field explains that the key is copied from the claude.ai
  browser cookie and used only to read usage — never sent to api.anthropic.com, never
  signing a request — and, on a local router, that it lives in a Keychain entry apart from
  the credential. *Why:* a reader pasting a browser cookie needs to know what it is for.
  *Regression:* yes — check "a hosted router speaks of its deployment, not of this Mac, and a
  local one as it always did". *Source:* `assets/index.html:553-555`,
  `assets/index.html:2294-2295`, `src/accounts.rs:396-401`.

### Accounts: adding

- **ST-C39** A visible "Add account" button opens the add form in place. *Why:* today it is
  a "+ add an account" link toggling a hidden form. *Regression:* no. *Source:*
  `assets/index.html:563-564`, `assets/index.html:2196`, `docs/web-console-carbon.md:376`.
- **ST-C40** Each way of adding an account is a choice carrying its own explanation where it
  is made — sign in from here (recommended), paste a token, or a separate Claude Code
  profile — and on a local router sign-in comes first and is chosen by default. *Why:* so
  that "`claude setup-token` is explained where the choice is made". *Regression:* yes —
  check "a hosted router adds an account by pasted token only, and a local one every way".
  *Source:* `assets/index.html:567-571`, `assets/index.html:2183-2194`,
  `docs/web-console-carbon.md:377-378`, `scripts/check-dashboard-ui.js:371-383`.
- **ST-C41** A name is required before anything is sent, and the form says so beside the
  Name field. *Why:* the router turns an empty name into a made-up id for a token or a
  profile, and refuses one for a sign-in ("give the account a name first"). *Regression:* no.
  *Source:* `assets/index.html:2232-2233`, `src/login.rs:321-324`, `src/accounts.rs:625-642`.
- **ST-C42** Adding never replaces an existing account: the form works out the id the
  router will give the name — lowercase ASCII letters and digits joined by single hyphens, at
  most 48 characters — and when that is an existing account's id, says so beside the Name
  field and sends nothing. *Why:* the router upserts on that id, so a new "Work" beside an
  existing "work" replaces that account's credential and source, and a new "Default" turns
  the built-in account into a token account. *Regression:* no. *Source:* `src/api.rs:1383`,
  `src/accounts.rs:526-541`, `src/accounts.rs:625-642`, `src/login.rs:142`,
  `src/login.rs:252-261`. *Today:* broken — nothing checks, and the existing account is
  overwritten.
- **ST-C43** The sign-in choice's button reads "Start sign-in", and its explanation says
  that the router runs `claude setup-token` and issues a long-lived (one-year) credential
  that never needs refreshing, and to sign in as the account being added, not the one
  already in use. *Why:* signing in as the current account only adds a second credential for
  it. *Regression:* no. *Source:* `assets/index.html:582-584`, `assets/index.html:2187-2188`,
  `src/login.rs:1-12`.
- **ST-C44** The console never opens the sign-in page by itself: it shows the whole link,
  wrapped so it can be read and copied, an "Open sign-in page" button that opens it in a new
  tab with no opener or referrer, and a "Copy link" button that says when copying failed and
  to select the link instead. *Why:* "Nothing opens on its own — use the button, or copy the
  link into whichever browser is signed into that account." *Regression:* no. *Source:*
  `assets/index.html:585-594`, `assets/index.html:2198-2202`, `assets/index.html:2210-2216`,
  `src/login.rs:116-118`.
- **ST-C45** While a sign-in runs, the form says its stage — starting, waiting for the
  reader to sign in, signed in (as whom), or failed with the router's reason — reading the
  flow's status about every 1.5 s, ignoring a single failed read, and stopping when the flow
  ends. *Why:* the flow is a terminal program the router drives, and a blip in reading its
  status is not its failure. *Regression:* no. *Source:* `assets/index.html:2204-2229`,
  `assets/index.html:2234-2244`, `src/login.rs:22-32`.
- **ST-C46** Only one sign-in runs at a time, so the form resumes one already running —
  showing its link and stage as soon as it opens — instead of offering a second start the
  router refuses; a status naming no flow (phase failed, no name, no error) is no flow and
  never shown as a failure. *Why:* `setup-token` binds a fixed local callback port, and the
  flow outlives the page that started it. *Regression:* no. *Source:*
  `src/login.rs:46-58`, `src/login.rs:325-331`, `assets/index.html:2204-2244`. *Today:*
  broken — a reload mid-flow loses the link, and Start sign-in then reads "a sign-in is
  already in progress".
- **ST-C47** Of what the sign-in status carries, the console shows only the link, the
  stage, the account and the error; the terminal `tail` is never rendered. *Why:* the tail
  is the last 400 characters of the terminal the token was printed on. *Regression:* no.
  *Source:* `src/login.rs:46-58`, `src/login.rs:106-114`, `src/login.rs:198-201`.
- **ST-C48** The token field explains where it is filled in how to get a token — run
  `claude setup-token` signed in as the account to add, in a terminal on this machine (on a
  hosted router, on any machine) — and that a `sk-ant-sid…` session key is a browser cookie
  that will not authenticate; the router's prefix rules are checked before sending. *Why:*
  pasting the cookie is the common mistake, and the add path stores any token unchecked.
  *Regression:* yes — check "a hosted router speaks of its deployment, not of this Mac, and a
  local one as it always did". *Source:* `assets/index.html:573-580`,
  `assets/index.html:2296-2298`, `src/api.rs:1300-1310`, `src/api.rs:1388-1395`. *Today:*
  broken — a pasted session key is added as an account without complaint.
- **ST-C49** When a token account's name is an email address, the Name field says it
  becomes the account's identity. *Why:* a long-lived token cannot report its account, so
  the router takes an email-shaped name as the only signal it has. *Regression:* no.
  *Source:* `src/api.rs:1396-1402`.
- **ST-C50** A pasted token leaves its field once the router has stored it, and stays in
  place beside a refusal. *Why:* Northstar §9 keeps the text on a refusal, and a stored
  secret has no reason to stay on screen. *Regression:* no. *Source:*
  `assets/index.html:2261-2266`, `../northstar/docs/13-console-contracts.md:368-369`.
  *Today:* broken — the field is emptied in a `finally`, so a refused token has to be pasted
  again.
- **ST-C51** The profile choice shows the command the router returns in a copyable block,
  says the page will notice when the login is done, re-checks the profile every 3 s until it
  reports logged in, then says who it signed in as and closes. *Why:* the login is a
  terminal program, and the page finishes the flow by itself. *Regression:* no. *Source:*
  `assets/index.html:2189-2192`, `assets/index.html:2249-2260`, `src/api.rs:1406-1418`,
  `src/api.rs:1429-1467`.
- **ST-C52** On a hosted router a pasted token is the only way to add an account: sign-in
  and profile are not offered, and the form is drawn for a token from the start. *Why:* a
  hosted router never runs `claude`, so it can neither sign in nor keep a profile, and it
  refuses both. *Regression:* yes — check "a hosted router adds an account by pasted token
  only, and a local one every way". *Source:* `assets/index.html:2308-2317`,
  `src/hosted.rs:28-31`, `src/hosted.rs:78-85`, `scripts/check-dashboard-ui.js:371-383`.

### Claude Code

- **ST-C53** The Claude Code page is not offered on a hosted router, and nothing it shows
  is read there. *Why:* none of it applies to a router with no Mac behind it; its setup
  routes refuse there. *Regression:* yes — check "a hosted router hides what only a Mac has,
  and a local one keeps all of it". *Source:* `assets/index.html:602`,
  `assets/index.html:2049-2051`, `assets/index.html:2300-2302`,
  `scripts/check-dashboard-ui.js:343-369`.
- **ST-C54** Start at login is a switch with its explanation ("Runs in the background and
  restarts if it crashes"); where the platform has no supported service manager the switch is
  replaced by "not supported on <platform> yet". *Why:* a switch that could only fail is not
  offered. *Regression:* no. *Source:* `assets/index.html:2056-2060`, `src/setup.rs:298-302`.
- **ST-C55** Route every project is a switch that names the global settings file it
  writes. *Why:* the reader sees which file changes. *Regression:* no. *Source:*
  `assets/index.html:2062-2065`, `src/setup.rs:304-306`.
- **ST-C56** Beside the routing controls the page says that only `env.ANTHROPIC_BASE_URL`
  is ever written, that everything else in the settings file is left as it is, and that
  turning routing off puts back what was there before unless it has since been changed by
  hand, in which case it is left alone. *Why:* it is the router's promise about a file it does
  not own. *Regression:* no. *Source:* `assets/index.html:612-614`, `src/main.rs:81-84`.
- **ST-C57** A routing change reports the router's own sentence for what it did — added,
  replaced and will restore, adopted, already pointing here, restored, removed, left alone
  because it now reads something else, or was not managed. *Why:* each is a different fact
  about the reader's file. *Regression:* no. *Source:* `src/setup.rs:369-391`,
  `assets/index.html:2099`.
- **ST-C58** Each routed project is listed with its path and a Stop routing button that
  sends the project's root, derived from the stored settings path by removing
  `/.claude/settings.local.json` with either separator. *Why:* "A managed project record
  stores the settings file; the API wants the project root." *Regression:* no. *Source:*
  `assets/index.html:2087-2091`, `assets/index.html:2119-2120`, `assets/index.html:2129-2132`.
- **ST-C59** Route a single project takes a project directory, and when the router refuses
  the path (not a directory) shows its sentence and keeps what was typed. *Why:* a refused
  path must be fixable in place. *Regression:* no. *Source:* `assets/index.html:606-611`,
  `assets/index.html:2121-2126`, `src/setup.rs:362-366`. *Today:* broken — a refused path
  reads "done" and the field is cleared.
- **ST-C60** The wrapper section, "Type into editor sessions", appears when an editor
  extension is found, explains that it lets the live view send messages to sessions VS Code
  starts and takes effect only for sessions started afterwards, and warns in words when the
  wrapper binary is missing next to agent-router. *Why:* sessions already open keep their
  old stdio, and a missing binary means nothing can be installed. *Regression:* no.
  *Source:* `assets/index.html:2067-2072`, `src/shim.rs:295-304`.
- **ST-C61** The automatic wrapper switch explains that every extension update ships a
  fresh, unwrapped binary which the router wraps within seconds, shows the last thing it did
  and when, can always be turned off, and can be turned on only while the wrapper binary is
  present. *Why:* turning it on without a binary could only fail. *Regression:* no.
  *Source:* `assets/index.html:2073-2077`, `src/shim.rs:310-319`.
- **ST-C62** Each extension binary is listed with its extension name and its state —
  installed, not installed, needs reinstalling — and an Install or Remove button that sends
  the path exactly as the listing gave it; Install is offered only while the wrapper binary
  is present, Remove always. *Why:* "Only ever touch a path the listing itself produced."
  *Regression:* no. *Source:* `assets/index.html:2078-2084`, `assets/index.html:2103-2108`,
  `src/shim.rs:321-326`.
- **ST-C63** A wrapper change says what follows: "Installed — restart a session to use
  it", "Removed", "On — new extension versions are wrapped as they arrive", "Off — the buttons
  below still work"; a refusal shows the router's `reason`. *Why:* a wrapper only reaches new
  sessions, which the reader must be told. *Regression:* no. *Source:*
  `assets/index.html:2103-2114`, `src/shim.rs:311-339`.
- **ST-C64** The page's heading says whether the login item is on and whether every
  project or only some are routed. *Why:* the section's state at a glance. *Regression:* no.
  *Source:* `assets/index.html:2093-2094`.

### Network

- **ST-C65** The Network page and its switch keep the name "Listen on your network". *Why:*
  both phone apps and the router's own help tell the reader to "turn on Listen on your
  network" in the dashboard's settings. *Regression:* no. *Source:*
  `assets/index.html:630`, `ios/AgentRouter/Sources/Views/RouterFormView.swift:49`,
  `android/app/src/main/java/ca/whnet/agentrouter/ui/RouterFormScreen.kt:218`,
  `src/main.rs:76-79`.
- **ST-C66** The switch shows whether the router listens beyond loopback as `/api/auth`
  reports it, and `/api/auth` is re-read after every answer to the switch, refusals
  included. *Why:* a refusal can follow a change that did happen — "listening changed, but it
  could not be saved for next time". *Regression:* no. *Source:*
  `assets/index.html:2362-2378`, `src/network.rs:321-347`, `docs/web-console-carbon.md:382-383`.
  *Today:* broken — after a refusal the page does not re-read, so the switch can show the old
  state over a socket that changed.
- **ST-C67** With the switch on, the page shows the address other devices reach the router
  at, copyable; with it off, it says "This Mac only" and where turning it on would put it.
  *Why:* it is what the reader types into a phone. *Regression:* no. *Source:*
  `assets/index.html:2380-2385`, `src/network.rs:253-263`, `docs/web-console-carbon.md:382-383`.
- **ST-C68** Turning the network on is offered only once a password is set, and until then
  the page says beside the switch to set a password first, since other devices are refused
  anyway; turning it off is always offered. *Why:* "Without a password every caller from
  another machine is refused anyway, so turning this on would be exposure for nothing"; the
  router refuses it too. *Regression:* no. *Source:* `assets/index.html:2367-2369`,
  `assets/index.html:2395-2397`, `src/network.rs:353-358`.
- **ST-C69** When `AGENT_ROUTER_BIND` is set in the router's environment, the page says
  beside the switch that the variable decides the next time the router starts. *Why:* "The
  environment wins over the setting at startup, so a router launched by hand with the
  variable set would come back up its own way whatever this says." *Regression:* no.
  *Source:* `assets/index.html:2398-2402`, `src/network.rs:286-294`.
- **ST-C70** The page explains that off means only this Mac is answered and on adds the
  local network, so the phones and a Claude Code elsewhere can reach the router, each asked
  for the password; and that a change takes effect at once, with nothing restarted and no
  session dropped, and is remembered across a reboot. *Why:* the switch adds or removes a
  second listener in the running router and stores the choice. *Regression:* no. *Source:*
  `assets/index.html:632-635`, `assets/index.html:2359-2361`.
- **ST-C71** Turning the network off asks first, and when the console is read from another
  machine the confirmation says this browser will lose the router and cannot turn it back on
  from there. *Why:* the switch closes the very listener that browser is connected through.
  *Regression:* no. *Source:* `assets/index.html:2366-2378`, `src/auth.rs:351`,
  `docs/web-console-carbon.md:448`. *Today:* broken — one click.
- **ST-C72** The Network page is not offered on a hosted router. *Why:* a hosted router
  listens where its deployment says, and refuses the switch. *Regression:* yes — check "a
  hosted router hides what only a Mac has, and a local one keeps all of it". *Source:*
  `assets/index.html:2300-2302`, `src/hosted.rs:26-27`, `src/hosted.rs:48`,
  `scripts/check-dashboard-ui.js:343-369`.

### Password

- **ST-C73** The password form has two fields, New password and Confirm, says "at least 8
  characters" before anything is typed, checks the length and that the two agree before
  sending, and submits on Enter in the second field. *Why:* nobody can see what was typed, and
  a typo would lock every other device out. *Regression:* yes — check "the password form
  builds in both homes without throwing". *Source:* `assets/index.html:2328-2347`,
  `assets/index.html:2354-2356`, `scripts/check-dashboard-ui.js:323-326`.
- **ST-C74** The length is counted the way the router counts it, in characters (Unicode
  scalar values) rather than UTF-16 units, and the password is sent exactly as typed, spaces
  included, never trimmed. *Why:* "a password is whatever was typed, spaces included"; four
  emoji are eight UTF-16 units but four characters, which the router refuses. *Regression:*
  no. *Source:* `src/auth.rs:389-392`, `assets/index.html:2335`. *Today:* broken — the page
  counts `.length`, so such a password passes the page, is refused with HTTP 200, and reads
  "set".
- **ST-C75** The button reads "Set password" when none is set and "Change password" when one
  is; changing never asks for the old password, and from this machine that is how a
  forgotten one is recovered. *Why:* "loopback may replace a forgotten password without
  knowing it". *Regression:* no. *Source:* `assets/index.html:640-642`,
  `assets/index.html:2332`, `src/auth.rs:16-19`.
- **ST-C76** The page explains who is asked for the password: on a local router, any device
  that is not this Mac — a browser through its usual prompt, with any username, and a Claude
  Code elsewhere through `ANTHROPIC_AUTH_TOKEN` — and that a forgotten one is replaced from
  here; on a hosted router, every caller. *Why:* the reader must know where it will be asked
  for before choosing it. *Regression:* yes — check "a hosted router speaks of its deployment,
  not of this Mac, and a local one as it always did". *Source:* `assets/index.html:640-642`,
  `assets/index.html:2292-2293`, `scripts/check-dashboard-ui.js:427-453`.
- **ST-C77** The heading says whether a password is set and how long ago it changed, or,
  when none is, that other devices are refused (network on) or that it only matters on the
  network (network off). *Why:* the only facts about the password that may be shown.
  *Regression:* no. *Source:* `assets/index.html:2418-2420`, `src/auth.rs:349-350`.
- **ST-C78** Remove password asks first; its confirmation says every other device is
  refused until a new one is set from this Mac and, when the console is read from another
  machine, that this browser is locked out at once. *Why:* with no password the guard refuses
  every caller from another machine, the reader's own browser included. *Regression:* no.
  *Source:* `assets/index.html:2350-2353`, `src/guard.rs:204-211`, `src/auth.rs:336-339`,
  `docs/web-console-carbon.md:385-386`, `docs/web-console-carbon.md:448`. *Today:* broken — one
  click.
- **ST-C79** A password pinned by the deployment reads "Set by the deployment. Change the
  Secret and restart the router." with nothing to set, change or remove. *Why:* the router
  answers any change with 409, and a change would not outlive the next start. *Regression:*
  yes — check "a pinned password is the deployment's: said so, with nothing to set, change or
  remove". *Source:* `assets/index.html:2322-2326`, `src/auth.rs:367-381`,
  `scripts/check-dashboard-ui.js:385-401`.
- **ST-C80** The first-boot password modal opens by itself only when all of these hold: the
  router listens on the network, no password is set, the console is read from this machine,
  the router is local, the password is not pinned, the reader has not dismissed it in this
  load, and Settings is not on screen. *Why:* "ask once, from this machine, while the network
  can reach us and nothing guards it yet"; a viewer on another machine cannot set one
  anyway. *Regression:* yes — checks "the password is asked for once, on first boot, from this
  machine only" and "a pinned password is the deployment's: said so, with nothing to set,
  change or remove". *Source:* `assets/index.html:2433-2440`,
  `scripts/check-dashboard-ui.js:302-321`, `scripts/check-dashboard-ui.js:389-396`,
  `docs/web-console-carbon.md:392`.
- **ST-C81** The first-boot modal says the router is reachable from the network, that other
  devices will be asked for this password and this Mac never is, and that until one is set
  other devices are refused; it offers Set password and Not now. *Why:* the reader is asked
  to choose without having opened anything. *Regression:* no. *Source:*
  `assets/index.html:515-524`, `assets/index.html:2349`.
- **ST-C82** Closing the first-boot modal in any way — Not now, Escape, or setting the
  password — counts as dismissed for the rest of the load. *Why:* it asks once.
  *Regression:* no. *Source:* `assets/index.html:2341`, `assets/index.html:2349`,
  `assets/index.html:2443`.
- **ST-C83** Once the password is set, both fields are emptied, the first-boot modal closes
  if open, and `/api/auth` is re-read; after a refusal both fields keep what was typed.
  *Why:* a set password must not linger on screen, and a refused one is fixed in place.
  *Regression:* no. *Source:* `assets/index.html:2337-2344`.

### Notifications

- **ST-C84** Notifications are read when their page opens, and a failed read says "Could
  not read notification settings" with the reason, never an empty form. *Why:* an unreadable
  state must not look like an unconfigured one. *Regression:* no. *Source:*
  `assets/index.html:946`, `assets/index.html:2446-2449`.
- **ST-C85** The heading names the key id and counts the registered phones, or says "no
  APNs key — phones cannot be told". *Why:* whether a waiting session can reach a phone at
  all. *Regression:* no. *Source:* `assets/index.html:2450-2452`.
- **ST-C86** Where the key came from is said: a key file the router took in names its path;
  with none, the page says to drop the `AuthKey_<key id>.p8` into `~/.agent-router` (locally)
  or into the deployment's push Secret (hosted), or to paste it here. *Why:* "The key as a
  file dropped beside the router's database: what was found, and what it still waits on.
  The paste form below is the other way in." *Regression:* yes — check "a hosted router's
  facts leave out its process id, and its key hint names the Secret". *Source:*
  `assets/index.html:2453-2459`, `src/notify.rs:1101`, `scripts/check-dashboard-ui.js:542-565`.
- **ST-C87** Team id, key id and bundle id are filled from what is stored; the `.p8` box is
  never filled, and while a key is stored says that a new one replaces it. *Why:* the
  private key never leaves the router. *Regression:* no. *Source:*
  `assets/index.html:2460-2463`, `src/notify.rs:1095-1101`, `docs/web-console-carbon.md:388`.
- **ST-C88** Save key (Replace key while one is stored) needs the team id, key id and `.p8`
  together and says so before sending; on a refusal — "that key does not sign" among them —
  the pasted key stays in its box beside the router's sentence. *Why:* the router proves a
  key signs before keeping it, and a refused paste that vanishes is a key lost silently.
  *Regression:* no. *Source:* `assets/index.html:2465-2468`, `src/notify.rs:1130-1145`,
  `docs/web-console-carbon.md:437`. *Today:* broken — a refusal reads "saved", and the redraw
  that follows empties the box.
- **ST-C89** Send a test is offered whenever either push service is configured, and its
  result — per phone, delivered or the error, or that no phone is registered — stays on
  screen after the page re-reads. *Why:* the test is the only way to watch a phone actually
  receive. *Regression:* no. *Source:* `assets/index.html:2469-2472`,
  `src/notify.rs:1112-1113`, `src/notify.rs:1250-1270`, `docs/web-console-carbon.md:390`.
  *Today:* broken — the result is overwritten by the redraw that follows it, and the test is
  offered only for an Apple key.
- **ST-C90** Remove key asks first, saying that phones stop being told until a key is stored
  again. *Why:* Apple lets a key be downloaded only once, so a removed key may be gone for
  good. *Regression:* no. *Source:* `assets/index.html:2473-2475`,
  `docs/web-console-carbon.md:448`. *Today:* broken — one click.
- **ST-C91** Each registered phone is a compact row: its name (or "phone"), the tail of its
  token, its environment, its platform, and its last result with when, or when it
  registered; with none, the page says how to register from the app (Settings →
  Notifications → Enable). *Why:* which phones will be told, and whether the last push
  reached them. *Regression:* no. *Source:* `assets/index.html:2476-2480`,
  `src/notify.rs:1082-1093`, `docs/web-console-carbon.md:389`.
- **ST-C92** The router's last push failure is shown, in the error tone. *Why:* a failed
  push says nothing on the phone. *Regression:* no. *Source:* `assets/index.html:2486`,
  `src/notify.rs:1115`.
- **ST-C93** Firebase is shown beside Apple — configured or not, its project, its key file
  and its last error — with a box that takes the service-account JSON whole. *Why:* the
  router already reports and accepts Firebase, for the Android app. *Regression:* no.
  *Source:* `src/notify.rs:1102-1111`, `src/notify.rs:1148-1188`,
  `docs/web-console-carbon.md:391`. *Today:* broken — the page ignores Firebase.
- **ST-C94** The page explains that the iPhone app is told the moment a session waits on the
  reader, through Apple's push service, which needs an APNs key from the developer account
  (Keys → + → Apple Push Notifications service), pasted once and kept in the router's secrets
  file, never in the database; on a hosted router, that it may also come from the
  deployment's push Secret. *Why:* what the key is for, and where it lives. *Regression:* yes
  — check "a hosted router speaks of its deployment, not of this Mac, and a local one as it
  always did". *Source:* `assets/index.html:625-627`, `assets/index.html:2288-2291`,
  `scripts/check-dashboard-ui.js:443`.

### Router

- **ST-C95** The Router page states where the console is read from ("Listening on", the
  page's own host), the version and, on a local router only, the process id; it re-reads the
  health each time it opens and says when that read failed. *Why:* a hosted router's process
  id is nothing a reader can act on, and a failed read kept for the visit hides a router that
  answers now. *Regression:* yes — check "a hosted router's facts leave out its process id,
  and its key hint names the Secret". *Source:* `assets/index.html:967-988`,
  `scripts/check-dashboard-ui.js:542-561`. *Today:* broken — a failed first health read is
  kept as `{}` for the life of the page, showing "—".
- **ST-C96** The connection snippet points `ANTHROPIC_BASE_URL` at the page's own origin and
  adds `"ANTHROPIC_AUTH_TOKEN": "<the router password>"` wherever the router asks the reader
  for it — a hosted router, or a page read from another machine — and never holds the
  password itself. *Why:* the page's origin is right behind an https ingress and through a
  port-forward, and the password stays with whoever knows it. *Regression:* yes — check "the
  connection snippet is the page's own origin, with the password wherever it is asked".
  *Source:* `assets/index.html:955-965`, `scripts/check-dashboard-ui.js:403-425`.
- **ST-C97** Until `/api/auth` has answered, the snippet is drawn as for this machine, and
  it is redrawn when the answer changes it. *Why:* "applyAuth draws it again if it was drawn
  before the page knew where it was read from." *Regression:* no. *Source:*
  `assets/index.html:956-966`, `assets/index.html:971-972`, `assets/index.html:2430-2432`.
- **ST-C98** The snippet says where it goes — a project's `.claude/settings.local.json` —
  and has a Copy button that says "Copied", or to select it by hand when the clipboard
  refuses. *Why:* the clipboard is refused in some frames and over plain HTTP on the LAN.
  *Regression:* no. *Source:* `assets/index.html:973-978`, `assets/index.html:985-987`.
- **ST-C99** The page says where the router's own settings come from: locally, environment
  variables read at startup (port, database location, upstream, body cap), with everything
  above stored locally; on a hosted router, the deployment, with everything kept in the
  router's data directory. *Why:* none of them can be changed from the console.
  *Regression:* yes — check "a hosted router speaks of its deployment, not of this Mac, and a
  local one as it always did". *Source:* `assets/index.html:620-621`,
  `assets/index.html:2286-2287`.


### Forges and the Library

Settings for the Library on a hosted router ([the Library](library.md)): where it imports from,
how a forge's webhook proves itself, the publish tokens, and whether publishes from machines go
live. Built 2026-10-08 as AGEN-0052, against the router of commits `be4d25f` to `15f3dda`.

- **ST-C104** On a hosted router Settings has Forges and Library, after Router. A local router
  has neither tab, and their addresses say the Library is a hosted router's. *Source:*
  `ui/src/model/settings.ts` (`settingsPages`), `ui/src/screens/Settings/Section.tsx`.
- **ST-C105** Forges lists each forge: its key, its kind (GitLab, GitHub), its address, whether a
  token is kept, and how many sources use it. It never shows a token. *Source:* `GET
  /api/library/forges`, which carries none.
- **ST-C106** Adding or changing a forge takes:
  - a key;
  - GitLab or GitHub;
  - an address;
  - a token, typed in a password field.

  On a change, a token left empty keeps the kept one, and "Remove the token" keeps none. The
  router's refusal shows beside the form in its words: an address in plain http, an address
  another forge has.
- **ST-C107** "Check" asks the forge and says what came back, as a heading over the router's
  own words. It is one of:
  - it answered;
  - it refused the token;
  - the token may not read this;
  - no such repository;
  - it could not be reached;
  - something unexpected.
- **ST-C108** "Remove" asks once more in a danger dialog (ST-C18). A forge a source uses is
  refused in the router's words, inside the dialog.
- **ST-C109** Library → Sources lists each source:
  - its forge, repository, ref and path ("the whole repository" when none);
  - whether its new versions go live or wait for approval;
  - what its last import came to, and when, or the forge's refusal, or "Never imported yet";
  - whether its webhook has a secret. The secret itself is never shown.
- **ST-C110** Adding a source takes:
  - a key;
  - a forge, chosen from the forges;
  - a repository;
  - a ref (`main` to start with);
  - a path;
  - whether its new versions go live, off by default.

  With no forge yet, it says to add one first.
- **ST-C111** "Import now" imports at once, and says what came of it: how many items are new,
  unchanged, refused and failed, at which commit, and which ones the repository no longer
  holds. The forge's refusal shows in its words.
- **ST-C112** A webhook secret is made on request. It is shown once, with the hook's URL, each
  with a copy button, and with how to give them to the forge. The words say any secret made
  before no longer works.
- **ST-C113** Publish tokens are listed by name: what each may publish, whether what it
  publishes goes live, when it was made, when last used, and when revoked. A value is never
  listed.
- **ST-C114** Making a token takes:
  - a name;
  - the kinds it may publish;
  - the items it alone may publish;
  - whether what it publishes goes live.

  Its value is shown once, with a copy button and the words that it is not shown again.
  "Revoke" asks once more in a danger dialog.
- **ST-C115** "Publishes from machines go live" is one switch. It is off (waiting for approval)
  until the owner turns it on, and the change is saved as it is made.
- **ST-C116** A failed read of the forges, the sources or the tokens is said in the router's
  words, never drawn as an empty list (SH-C22).

### A hosted router

- **ST-C100** Settings decides what to offer from `/api/auth`'s `mode`: `server` is a hosted
  router, and an answer without `mode` comes from a local router that predates the field and
  reads exactly as local. *Why:* the console reads what the router can do rather than
  assuming a Mac, and older routers keep working. *Regression:* yes — check "a hosted router
  hides what only a Mac has, and a local one keeps all of it". *Source:*
  `assets/index.html:2277-2282`, `src/auth.rs:352-356`, `scripts/check-dashboard-ui.js:328-335`,
  `docs/web-console-carbon.md:485-488`. *Under decision J:* the console ships inside the router it reads, so the older-router case here never reaches it; the rest of the rule stands, and the phones still meet older routers.
- **ST-C101** Until `/api/auth` has said which kind of router this is, Settings offers none
  of the machine-only controls: the Claude Code and Network pages, and the sign-in and
  profile ways to add an account. *Why:* offering them first would flash controls a hosted
  router refuses — the reason Northstar's shell holds its Sign in button back until `/auth`
  answers. *Regression:* no. *Source:* `../northstar/ui/src/app/Shell.tsx:57-60`,
  `assets/index.html:2507-2509`, `../northstar/docs/13-console-contracts.md:370-373`.
- **ST-C102** On a hosted router every sentence that speaks of "this Mac", "this machine",
  the Keychain, "stored locally" or `~/.agent-router` gives way to the deployment's wording,
  and a local router keeps its own. *Why:* "Wording that speaks of this Mac, as a hosted
  router says it. The markup keeps the local wording, and gets it back." *Regression:* yes —
  check "a hosted router speaks of its deployment, not of this Mac, and a local one as it
  always did". *Source:* `assets/index.html:2283-2307`, `scripts/check-dashboard-ui.js:427-453`.
- **ST-C103** A Settings address for a page a hosted router does not have (a bookmarked
  `/settings/network`) says why, in the router's sentence — "set those up on each machine" —
  never as a blank page or a not-found. *Why:* the router's refusal already says what to do
  instead. *Regression:* no. *Source:* `src/hosted.rs:26-27`,
  `../northstar/docs/13-console-contracts.md:332-333`.

## Serving

What the router must do to serve the console: its routes and the one guard layer around
them, the guard as a browser meets it, the headers, framing, embedding and the build, the
URLs that must keep answering, development, and what a hosted router changes.

### Routes and the guard layer

- **SV-C1** Every console route is registered on the one router before its single
  `guard::layer` — in the local and the hosted route sets alike — so the guard wraps the
  document and every asset exactly as it wraps `/api` and `/v1`. *Why:* an axum `.layer`
  wraps only the routes added before it, so a console route added after would skip the
  guard. *Regression:* no. *Source:* `src/main.rs:384-412`, `docs/web-console-carbon.md:252-253`.
- **SV-C2** The console never takes the fallback and never registers a root wildcard: the
  fallback stays the proxy, and every console path is named. *Why:* the fallback is how Claude
  Code reaches `/v1/*`, and a wildcard would catch it and break every agent. *Regression:*
  no. *Source:* `src/main.rs:409-411`, `src/proxy.rs:247-250`,
  `docs/web-console-carbon.md:257-258`.
- **SV-C3** Every path the app can show answers the document on a reload — `/`, `/live`,
  `/settings` and each `/settings/<section>` — and a test holds the client's routes and the
  server's in step. *Why:* a path the server does not name falls to the proxy, which answers
  a reload with 404 "agent-router: not an API path". *Regression:* no. *Source:*
  `docs/web-console-carbon.md:184-188`, `docs/web-console-carbon.md:254`,
  `src/proxy.rs:247-250`.
- **SV-C4** One document serves every route, so one frame policy and one CSP cover the
  Overview, Sessions and Settings. *Why:* today the two pages carry different policies, and
  the one that approves prompts carries none. *Regression:* no. *Source:*
  `docs/web-console-carbon.md:267-272`, `src/api.rs:25-39`, `src/live.rs:1204-1215`.
- **SV-C5** `/assets/{*path}` serves the embedded, hashed files and answers a plain 404 for
  a file it does not hold, never the document. *Why:* a tab left open across a deploy must
  see its missing chunk fail as a failure, which is what makes it reload. *Regression:* no.
  *Source:* `docs/web-console-carbon.md:255`, `docs/web-console-carbon.md:278-279`.
- **SV-C6** The document refers to its assets by absolute path from the app's base, so a
  reload on `/settings/<section>` or `/live?session=` still loads them. *Why:* a relative
  asset path resolves under `/settings/` and misses. *Regression:* no. *Source:*
  `docs/web-console-carbon.md:252-256`, `docs/web-console-carbon.md:464-465`.
- **SV-C7** `/favicon.svg` keeps answering as `image/svg+xml`, and the document links it
  rather than Northstar's `/favicon.ico`. *Why:* the inline SVG keeps the binary
  self-contained and the tab identifiable, and `/favicon.ico` has no route, so it falls to
  the proxy's 404. *Regression:* no. *Source:* `src/api.rs:41-48`, `src/main.rs:386`,
  `assets/index.html:7`, `../northstar/ui/index.html:8`.
- **SV-C8** The network listener serves the very same app as the loopback one, so the
  console off the machine sits behind the same guard. *Why:* the wider bind is a second
  socket serving `app.clone()`, not a second app. *Regression:* no. *Source:*
  `src/main.rs:433-437`.
- **SV-C9** The console replaces the old pages in one cut: `/` and `/live` serve it, both
  `include_str!` pages are gone with the code that served them, and nothing answers under
  `/next/`. *Why:* decision J, a clean cut with no `/next/` period and no old code kept; the
  console ships inside the router it talks to, so it never meets an older one. *Regression:*
  no. *Source:* plan, decision J; `src/api.rs:23`, `src/live.rs:1202` (the two pages).

### The guard, as a browser meets it

- **SV-C10** A local router never asks a browser on its own machine, over loopback, for a
  password. *Why:* "whoever sits here already holds the Keychain", and the menubar app, the
  extension and the local browser all arrive over loopback. *Regression:* no. *Source:*
  `src/auth.rs:3-9`, `src/guard.rs:197-201`.
- **SV-C11** A browser on another machine is challenged with HTTP Basic — a 401 carrying
  `WWW-Authenticate: Basic realm="agent-router", charset="UTF-8"` — gives any username, and
  is let in by the password; the console has no login page and no sign-in or log-out action
  of its own. *Why:* the browser handles Basic itself, so Northstar's `/login` route and its
  session actions are left out. *Regression:* no. *Source:* `src/guard.rs:212-244`,
  `src/auth.rs:138-153`, `docs/web-console-carbon.md:547-548`,
  `docs/web-console-carbon.md:556-558`.
- **SV-C12** Before any password exists, a browser on another machine is refused with 403
  and "no password set — set one from the dashboard on the machine running the router", with
  no prompt, so the first password can only be set from the machine itself. *Why:* nothing
  off the machine can reach `POST /api/auth` until a password exists — the safe default, and
  the recovery path. *Regression:* no. *Source:* `src/guard.rs:204-211`, `src/auth.rs:16-19`.
- **SV-C13** The document and its assets are served only to a `Host` the guard accepts:
  `localhost` or a loopback address; with the network on, also a bare IP literal; on a hosted
  router, also the names it was given. Any other name is refused, so on the LAN the console
  is reached by IP address, never by a name. *Why:* DNS rebinding points a name at
  127.0.0.1 and passes the socket and peer checks; only the Host check fails it, and it keeps
  that purpose when widened because rebinding needs a name. *Regression:* yes — test
  `rejects_rebinding_hosts`. *Source:* `src/guard.rs:9-13`, `src/guard.rs:56-69`,
  `src/guard.rs:108-132`, `src/guard.rs:330-342`.
- **SV-C14** Hosts are compared parsed, never by pattern: `127.0.0.1.evil.com` and
  `localhost.evil.com` are refused, and a hosted router's names match exactly,
  case-insensitively, port ignored and one trailing dot dropped, never as a suffix. *Why:*
  those are ordinary registrable names an attacker can point at 127.0.0.1. *Regression:* yes
  — tests `rejects_rebinding_hosts` and `a_hosted_router_answers_to_its_names_only`.
  *Source:* `src/guard.rs:56-84`, `src/guard.rs:255-277`.
- **SV-C15** Every write the console makes is a same-origin request, so its `Origin` names
  the same place as its `Host`; a cross-site write and a `null` origin are refused with 403.
  *Why:* another site can make a browser send a write with the router's cached Basic
  credentials attached, and every other layer passes it. *Regression:* yes — test
  `same_origin_writes_pass_and_cross_site_ones_do_not`. *Source:* `src/guard.rs:19-21`,
  `src/guard.rs:134-174`, `src/guard.rs:287-313`.
- **SV-C16** Nothing the console reads with GET, HEAD or OPTIONS changes state. *Why:* reads
  are not origin-checked — "the browser will not show another site what they return" — so a
  state change behind a GET could be triggered by any page. *Regression:* no. *Source:*
  `src/guard.rs:134-144`.
- **SV-C17** The extension's fallback frame keeps `allow-same-origin`, so the console inside
  it writes with the router's own origin. *Why:* a sandboxed frame without it sends
  `Origin: null`, which the guard refuses. *Regression:* no. *Source:*
  `vscode/src/dashboard.ts:21-24`, `src/guard.rs:139-140`, `docs/hosting-on-kubernetes.md:220-223`.
- **SV-C18** `/_router/health` stays outside the password and answers the pid, version and
  build; the console reads the router's version there, and nothing else it needs is exempt (a
  hosted router's agent installer is open too, SV-C40, but the console never reads it).
  *Why:* the handover probe and the kubelet read health before they have anything to
  authenticate with. *Regression:* no. *Source:* `src/guard.rs:197-201`,
  `src/api.rs:1510-1520`, `assets/index.html:969`.
- **SV-C19** The console never sends the reader to another origin — not from `127.0.0.1` to
  `localhost`, not to a LAN address, not to a hosted router's `public_url`. *Why:* the
  extension opens `http://127.0.0.1:<port>`, and its fallback frame's policy admits exactly
  that origin. *Regression:* no. *Source:* `vscode/src/router.ts:29`,
  `vscode/src/dashboard.ts:21-24`.

### Headers

- **SV-C20** The document is served with `Cache-Control: no-store` on every console route.
  *Why:* "The page ships inside the binary and changes with every build, so a cached copy is
  a stale copy — and a phone holding one looks exactly like 'it's not working'."
  *Regression:* yes — the comment at `src/live.rs:1205-1206`. *Source:*
  `src/live.rs:1204-1215`, `docs/web-console-carbon.md:254`. *Today:* broken — `/` sends no
  `Cache-Control` at all.
- **SV-C21** The hashed assets are served with `Cache-Control: public, max-age=31536000,
  immutable`. *Why:* a hashed file never changes under its name, and a phone on the LAN
  should not fetch it twice. *Regression:* no. *Source:* `docs/web-console-carbon.md:255`.
- **SV-C22** Every response carries `X-Content-Type-Options: nosniff`, and every embedded
  file is served with the type its extension names: the document as
  `text/html; charset=utf-8`, scripts as `text/javascript`, styles as `text/css`, fonts as
  `font/woff2`, icons as `image/svg+xml`. *Why:* under `nosniff` a mistyped script or
  stylesheet is refused rather than guessed at. *Regression:* no. *Source:*
  `docs/web-console-carbon.md:260`, `src/api.rs:41-47`. *Today:* broken — the router sends
  `nosniff` nowhere.
- **SV-C23** The document carries `Content-Security-Policy: default-src 'self'; script-src
  'self'; style-src 'self'; font-src 'self'; img-src 'self' data:; connect-src 'self';
  object-src 'none'; base-uri 'none'; form-action 'none'` plus the frame policy, in
  Report-Only mode until the first build shows that no dependency injects a `<style>`
  element and that Vite emits no inline script, and enforced from then on. *Why:* this is the
  page that approves prompts and can switch a session to bypass permissions, and on a hosted
  router it is served on a public name. *Regression:* no. *Source:*
  `docs/web-console-carbon.md:259-266`, `docs/web-console-carbon.md:489`,
  `docs/web-console-carbon.md:652-653`.
- **SV-C24** No inline script ships: the theme's before-first-paint snippet is a
  same-origin file the document loads first, not Northstar's inline `<script>`. *Why:*
  `script-src 'self'` refuses inline script, and Northstar's `index.html` sets its theme with
  one. *Regression:* no. *Source:* `../northstar/ui/index.html:9-18`,
  `docs/web-console-carbon.md:261`, `docs/web-console-carbon.md:652-653`.
- **SV-C25** `base-uri 'none'` and `form-action 'none'` stay, so every form's submit is
  handled in script and no form ever navigates. *Why:* today's dashboard sends both, and a
  natively submitted form would be refused by the policy. *Regression:* no. *Source:*
  `src/api.rs:31-34`, `docs/web-console-carbon.md:262-263`.
- **SV-C26** The console loads nothing from any other origin — no font service, no CDN, no
  analytics — because the fonts are self-hosted and everything else is embedded. *Why:* the
  README promises no network, and the policy's sources are all `'self'`. *Regression:* no.
  *Source:* `README.md:111-113`, `docs/web-console-carbon.md:176`,
  `docs/web-console-carbon.md:304-307`.

### Framing

- **SV-C27** Only VS Code's own schemes may frame the console — `frame-ancestors
  vscode-webview: vscode-file:` on every route — and this is checked in VS Code's Simple
  Browser and in the extension's fallback frame before it ships. *Why:* the console approves
  prompts and can switch a session to bypass permissions, and any web page can frame
  `http://127.0.0.1:8787/live`, because the guard's Host and peer checks both pass for it; no
  web page can use those two schemes. *Regression:* no. *Source:*
  `docs/web-console-carbon.md:267-275`, `docs/web-console-carbon.md:436`,
  `docs/web-console-carbon.md:633`, `docs/web-console-carbon.md:654`. *Today:* broken — `/live`
  sends no frame policy, so any page can frame it.
- **SV-C28** `X-Frame-Options` is not sent. *Why:* it cannot express an allow-list, and
  `DENY` refuses the frame in which VS Code's Simple Browser and the extension's fallback
  show the console on every editor before 1.109, while the extension supports VS Code from
  1.94. *Regression:* no. *Source:* `docs/web-console-carbon.md:268-275`, `src/api.rs:25-37`,
  `vscode/package.json:17`, `vscode/src/dashboard.ts:1-3`. *Today:* broken — `/` sends
  `X-Frame-Options: DENY` and `frame-ancestors 'none'`, which by the headers' logic breaks
  Simple Browser before 1.109 (not yet run).

### Embedding and build

- **SV-C29** The console is embedded with `rust-embed` over `ui/dist`, with
  `allow_missing`, and the product stays one binary that reads no console file from disk at
  run time. *Why:* Decision B and the one-static-binary promise. *Regression:* no. *Source:*
  `docs/web-console-carbon.md:252`, `docs/web-console-carbon.md:630`, `ARCHITECTURE.md:455`.
- **SV-C30** A debug build with no `ui/dist` serves a placeholder page that names the
  command that builds the console, and prints a `cargo:warning`. *Why:* a change to the Rust
  alone must not need Node, and a blank page would read as a broken router. *Regression:* no.
  *Source:* `docs/web-console-carbon.md:292-295`.
- **SV-C31** A release build with no `ui/dist` fails unless `AGENT_ROUTER_ALLOW_NO_UI=1` is
  set, and `build.rs` declares `rerun-if-changed=ui/dist`. *Why:* a release must never ship
  the placeholder, and a rebuilt console must rebuild the binary that embeds it.
  *Regression:* no. *Source:* `docs/web-console-carbon.md:292-295`,
  `docs/web-console-carbon.md:630`, `build.rs:1-40`.
- **SV-C32** The console is built once, before any cargo step — by `build-target.sh`, and by
  a CI job whose `ui/dist` the binary jobs take as an artifact — never committed and never
  built from `build.rs`. *Why:* targets build in parallel and would race on `ui/dist`, a
  committed `dist` churns history and collides between sessions, and pnpm in `build.rs`
  would put Node on every cargo call. *Regression:* no. *Source:*
  `docs/web-console-carbon.md:296-301`, `docs/web-console-carbon.md:630`.
- **SV-C33** A change to the console alone still bumps `CARGO_PKG_VERSION`. *Why:*
  `--ensure`, which the extension runs in every window it opens, replaces a running router
  only with a newer version, so a console-only change would never reach a router already
  serving. *Regression:* no. *Source:* `docs/web-console-carbon.md:308-310`,
  `src/main.rs:134-151`.
- **SV-C34** The app reloads itself, once, on `vite:preloadError`, which fires because a chunk
  the binary does not hold answers a plain 404. *Why:* a tab left open across a handover asks
  the new binary for chunks it does not have, and the screens are lazy chunks as well as the
  charts (the route table keeps Northstar's lazy screens), so any of them can be missing.
  *Regression:* no. *Source:* `docs/web-console-carbon.md:278-279`,
  `docs/web-console-carbon.md:559-560`, `src/main.rs:418-430`.

### URLs that must keep working

- **SV-C35** `/` keeps answering with the Overview from every launcher: the menubar's "Open
  dashboard" (`http://127.0.0.1:<port>`, no path), the extension's command and its status-bar
  item (`http://127.0.0.1:<port>/`), and the iPhone's "Open the dashboard in Safari" (the
  router's base URL). *Why:* every external consumer opens a bare `/` or `/live`.
  *Regression:* no. *Source:* `menubar/src/main.rs:253-256`, `vscode/src/dashboard.ts:8-11`,
  `vscode/src/extension.ts:18`, `vscode/src/status.ts:16`,
  `ios/AgentRouter/Sources/Views/RouterSettingsView.swift:146-148`,
  `docs/web-console-carbon.md:64-74`.
- **SV-C36** `/live` keeps answering with Sessions, and `/live?session=<id>` opens that
  session: a reload lands back on it, Back returns to the list, and an id no longer listed
  shows the list. *Why:* "The open session lives in the URL, so a refresh comes back to it",
  the extension opens `/live`, and a bookmark carries the id. *Regression:* no. *Source:*
  `assets/live.html:1140-1145`, `assets/live.html:1397-1401`, `assets/live.html:1545-1552`,
  `assets/live.html:1681-1689`, `vscode/src/extension.ts:19`.
- **SV-C37** A dashboard address in the old hash form, `/#a=…&p=…&s=…&r=…&o=…`, opens the console at `/` unscoped; nothing reads or rewrites the old hash. *Why:* the clean cut (decision J) keeps no compatibility code for the old pages' addresses. *Regression:* no. *Source:* `assets/index.html:908-925` (the old spelling); plan, decision J.
- **SV-C38** No other console address is promised outside the app: push payloads carry no
  URL, and the phones route a tap natively by session id. *Why:* the rewrite is free to
  shape every other path. *Regression:* no. *Source:* `docs/web-console-carbon.md:64-74`.

### Development

- **SV-C39** The Vite dev server listens on loopback only and proxies `/api` to a scratch
  router on a free port, never to the router on 8787, and a store copied for it is first
  scrubbed of its stored bind, password, wrapper switch and registered phones. *Why:* the
  guard trusts the connecting peer, so a dev server on the LAN would make every LAN browser
  look local and skip the password, and a copied store would act on the owner's network and
  phones. *Regression:* no. *Source:* `docs/web-console-carbon.md:280-285`.

### A hosted router

- **SV-C40** On a hosted router every caller presents the password, loopback included — a
  `kubectl port-forward` to localhost too — and only `/_router/health` and the agent's
  installer are open: `/agent/install.sh`, `/agent/install.ps1` and the files they fetch, by
  exact path (`src/downloads.rs`). *Why:* in a pod, loopback is a port-forward or a mesh
  sidecar, not the owner at the keyboard; the installer holds nothing secret, and a machine
  being connected has no password to present until the script asks for it.
  *Regression:* no. *Source:* `src/guard.rs:23-27`, `src/guard.rs:197-201`,
  `src/auth.rs:24-26`, `src/main.rs:505-510`, `docs/hosting-on-kubernetes.md:149-151`.
- **SV-C41** A hosted router answers to the names it was given (`--allow-host` and the
  `--public-url` host), to IP literals for its probes and to `localhost` for a port-forward;
  any other name gets a 403 that names what it does answer to. *Why:* it is reached by name
  through its ingress, and every caller there presents the password a rebinding page could
  not. *Regression:* yes — test `a_hosted_router_answers_to_its_names_only`. *Source:*
  `src/guard.rs:71-84`, `src/guard.rs:108-127`, `docs/hosting-on-kubernetes.md:141-148`.
- **SV-C42** On a hosted router TLS ends in front of the router and the console is served
  over https on a public name: the CSP and the frame policy are not optional there, and the
  console's own addresses come from the page's origin, which is right behind the ingress.
  *Why:* "whatever presents these names to clients must terminate TLS", and a page served
  publicly is the one most worth framing. *Regression:* no. *Source:* `src/main.rs:556-559`,
  `docs/web-console-carbon.md:489`, `assets/index.html:956-959`.
- **SV-C43** On a hosted router the machine routes stay registered and answer in the shapes
  the installed phones decode, refusing each action with HTTP 409 or 503 and `{ok: false,
  error}`; the console shows that `error` as the router wrote it. *Why:* the iPhone app
  requires every field of `/api/setup` and `/api/setup/shim`, and the sentence says what to
  do instead. *Regression:* yes — check "a per-session action a hosted router refuses says
  what it said". *Source:* `src/hosted.rs:1-13`, `src/hosted.rs:36-68`,
  `scripts/check-live-ui.js:1053-1091`.
- **SV-C44** On a hosted router with no machine connected, Sessions lists nothing and says why,
  with the commands that connect one (SE-C187) — never "No sessions in the last week", and warns
  of no unwrapped editor. *Why:* the empty
  list is a fact about the router, not about the reader's week. *Regression:* yes — check "a
  hosted router says why its list is empty, and warns of no wrapper". *Source:*
  `assets/live.html:415-421`, `assets/live.html:624-655`, `src/hosted.rs:32-33`,
  `src/hosted.rs:126-128`, `scripts/check-live-ui.js:1095-1115`.
